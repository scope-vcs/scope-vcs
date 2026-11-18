use crate::{
    STDERR_DIAGNOSTIC_BYTES, configure_process_group,
    lifecycle::ChildGuard,
    stdio::{
        StdoutActivity, diagnostic_suffix, join_reader, join_writer, read_stderr_diagnostic,
        read_stdout,
    },
};
use std::{
    io::{Read, Write},
    process::{ChildStdin, Command, ExitStatus, Output, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use tokio::sync::watch;

#[derive(Clone, Debug)]
pub struct ProcessCancellation {
    cancelled: watch::Sender<bool>,
}

impl ProcessCancellation {
    pub fn new() -> Self {
        let (cancelled, _) = watch::channel(false);
        Self { cancelled }
    }

    pub fn cancel(&self) {
        self.cancelled.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.cancelled.borrow()
    }

    pub async fn cancelled(&self) {
        let mut cancelled = self.cancelled.subscribe();
        loop {
            if *cancelled.borrow_and_update() {
                return;
            }
            if cancelled.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Default for ProcessCancellation {
    fn default() -> Self {
        Self::new()
    }
}

enum Expiry {
    Timeout(Duration),
    StdoutIdle(Duration),
}

impl Expiry {
    fn into_error(self, action: &str, stderr: &[u8]) -> ProcessError {
        let action = action.to_string();
        let diagnostic = diagnostic_suffix(stderr, STDERR_DIAGNOSTIC_BYTES);
        match self {
            Self::Timeout(timeout) => ProcessError::TimedOut {
                action,
                timeout_ms: timeout.as_millis(),
                diagnostic,
            },
            Self::StdoutIdle(idle) => ProcessError::Stalled {
                action,
                idle_ms: idle.as_millis(),
                diagnostic,
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessLimits {
    timeout: Duration,
    stdout_idle_timeout: Option<Duration>,
    max_stdout_bytes: Option<usize>,
}

impl ProcessLimits {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            stdout_idle_timeout: None,
            max_stdout_bytes: None,
        }
    }

    pub fn with_stdout_idle_timeout(mut self, stdout_idle_timeout: Duration) -> Self {
        self.stdout_idle_timeout = Some(stdout_idle_timeout);
        self
    }

    pub fn with_max_stdout_bytes(mut self, max_stdout_bytes: usize) -> Self {
        self.max_stdout_bytes = Some(max_stdout_bytes);
        self
    }

    fn expiry(&self, activity: &StdoutActivity) -> Option<Expiry> {
        if activity.elapsed() >= self.timeout {
            return Some(Expiry::Timeout(self.timeout));
        }
        self.stdout_idle_timeout
            .filter(|idle| activity.idle() >= *idle)
            .map(Expiry::StdoutIdle)
    }

    fn poll_interval(&self, activity: &StdoutActivity) -> Duration {
        self.timeout
            .saturating_sub(activity.elapsed())
            .min(Duration::from_millis(1))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("failed to run {action}: {source}")]
    Spawn {
        action: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{action} {pipe} pipe is unavailable")]
    PipeUnavailable { action: String, pipe: &'static str },
    #[error("{action} process I/O failed: {source}")]
    Io {
        action: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{action} process I/O thread panicked")]
    ThreadPanicked { action: String },
    #[error("{action} timed out after {timeout_ms}ms{diagnostic}")]
    TimedOut {
        action: String,
        timeout_ms: u128,
        diagnostic: String,
    },
    #[error("{action} produced no output for {idle_ms}ms{diagnostic}")]
    Stalled {
        action: String,
        idle_ms: u128,
        diagnostic: String,
    },
    #[error("{action} was cancelled")]
    Cancelled { action: String },
    #[error("{action} stdout exceeded {max_stdout_bytes} bytes{diagnostic}")]
    StdoutLimitExceeded {
        action: String,
        max_stdout_bytes: usize,
        diagnostic: String,
    },
}

#[derive(Debug)]
pub struct StreamedOutput<T> {
    pub status: ExitStatus,
    pub value: T,
    pub stderr: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum StreamingProcessError<E> {
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("process stdout consumer failed: {0}")]
    Consumer(E),
}

impl ProcessError {
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::TimedOut { .. } | Self::Stalled { .. })
    }

    pub fn is_stdout_limit(&self) -> bool {
        matches!(self, Self::StdoutLimitExceeded { .. })
    }
}

pub fn run(
    command: &mut Command,
    input: Option<Vec<u8>>,
    limits: ProcessLimits,
    action: &str,
) -> Result<Output, ProcessError> {
    run_inner(command, input, limits, action, None)
}

pub fn run_cancellable(
    command: &mut Command,
    input: Option<Vec<u8>>,
    limits: ProcessLimits,
    action: &str,
    cancellation: &ProcessCancellation,
) -> Result<Output, ProcessError> {
    run_inner(command, input, limits, action, Some(cancellation))
}

fn run_inner(
    command: &mut Command,
    input: Option<Vec<u8>>,
    limits: ProcessLimits,
    action: &str,
    cancellation: Option<&ProcessCancellation>,
) -> Result<Output, ProcessError> {
    if cancellation.is_some_and(ProcessCancellation::is_cancelled) {
        return Err(ProcessError::Cancelled {
            action: action.to_string(),
        });
    }
    let (child, stdin_writer) = spawn_with_stdin(command, StdinSource::from(input), action)?;
    wait_for_output(child, stdin_writer, limits, action, cancellation)
}

pub fn run_with_stdin_reader<R>(
    command: &mut Command,
    input: R,
    limits: ProcessLimits,
    action: &str,
) -> Result<Output, ProcessError>
where
    R: Read + Send + 'static,
{
    let (child, stdin_writer) =
        spawn_with_stdin(command, StdinSource::Reader(Box::new(input)), action)?;
    wait_for_output(child, stdin_writer, limits, action, None)
}

pub fn run_with_stdout<T, E, F>(
    command: &mut Command,
    input: Option<Vec<u8>>,
    limits: ProcessLimits,
    action: &str,
    consume: F,
) -> Result<StreamedOutput<T>, StreamingProcessError<E>>
where
    T: Send + 'static,
    E: Send + 'static,
    F: FnOnce(Box<dyn Read + Send>, ProcessCancellation) -> Result<T, E> + Send + 'static,
{
    let (mut child, stdin_writer) = spawn_with_stdin(command, StdinSource::from(input), action)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ProcessError::PipeUnavailable {
            action: action.to_string(),
            pipe: "stdout",
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ProcessError::PipeUnavailable {
            action: action.to_string(),
            pipe: "stderr",
        })?;
    let cancellation = ProcessCancellation::new();
    let consumer_cancellation = cancellation.clone();
    let activity = StdoutActivity::start();
    let stdout = activity.reader(stdout);
    let mut stdout_consumer = Some(thread::spawn(move || {
        consume(Box::new(stdout), consumer_cancellation)
    }));
    let stderr_reader =
        thread::spawn(move || read_stderr_diagnostic(stderr, STDERR_DIAGNOSTIC_BYTES));

    let mut status = None;
    let mut value = None;
    let (status, value) = loop {
        if let Some(finished) = stdout_consumer.take_if(|consumer| consumer.is_finished()) {
            match finished.join() {
                Ok(Ok(consumed)) => value = Some(consumed),
                Ok(Err(error)) => {
                    cancellation.cancel();
                    child.terminate_and_reap();
                    join_writer(stdin_writer, action).ok();
                    join_reader(stderr_reader, action).ok();
                    return Err(StreamingProcessError::Consumer(error));
                }
                Err(_) => {
                    cancellation.cancel();
                    child.terminate_and_reap();
                    join_writer(stdin_writer, action).ok();
                    join_reader(stderr_reader, action).ok();
                    return Err(ProcessError::ThreadPanicked {
                        action: action.to_string(),
                    }
                    .into());
                }
            }
        }
        if status.is_none() {
            status = match child.try_wait() {
                Ok(status) => status,
                Err(source) => {
                    cancellation.cancel();
                    child.terminate_and_reap();
                    join_writer(stdin_writer, action).ok();
                    if let Some(stdout_consumer) = stdout_consumer {
                        stdout_consumer.join().ok();
                    }
                    join_reader(stderr_reader, action).ok();
                    return Err(ProcessError::Io {
                        action: action.to_string(),
                        source,
                    }
                    .into());
                }
            };
        }
        let stdin_done = stdin_writer
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished);
        if stderr_reader.is_finished()
            && stdin_done
            && let Some(exit_status) = status
            && let Some(consumed) = value.take()
        {
            break (exit_status, consumed);
        }
        if let Some(expiry) = limits.expiry(&activity) {
            cancellation.cancel();
            child.terminate_and_reap();
            join_writer(stdin_writer, action).ok();
            if let Some(stdout_consumer) = stdout_consumer {
                stdout_consumer.join().ok();
            }
            let stderr = join_reader(stderr_reader, action).unwrap_or_default();
            return Err(expiry.into_error(action, &stderr).into());
        }
        thread::sleep(limits.poll_interval(&activity));
    };

    child.disarm();
    join_writer(stdin_writer, action)?;
    let stderr = join_reader(stderr_reader, action)?;
    Ok(StreamedOutput {
        status,
        value,
        stderr,
    })
}

enum StdinSource {
    Null,
    Bytes(Vec<u8>),
    Reader(Box<dyn Read + Send>),
}

impl From<Option<Vec<u8>>> for StdinSource {
    fn from(input: Option<Vec<u8>>) -> Self {
        input.map_or(Self::Null, Self::Bytes)
    }
}

type StdinWriter = thread::JoinHandle<std::io::Result<()>>;

fn spawn_with_stdin(
    command: &mut Command,
    stdin: StdinSource,
    action: &str,
) -> Result<(ChildGuard, Option<StdinWriter>), ProcessError> {
    command.stdin(match stdin {
        StdinSource::Null => Stdio::null(),
        StdinSource::Bytes(_) | StdinSource::Reader(_) => Stdio::piped(),
    });
    configure_process_group(command);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map(ChildGuard::new)
        .map_err(|source| ProcessError::Spawn {
            action: action.to_string(),
            source,
        })?;
    let stdin_writer = match stdin {
        StdinSource::Null => None,
        StdinSource::Bytes(input) => {
            let mut child_stdin = take_stdin(&mut child, action)?;
            Some(thread::spawn(move || {
                child_stdin.write_all(&input)?;
                child_stdin.flush()
            }))
        }
        StdinSource::Reader(mut input) => {
            let mut child_stdin = take_stdin(&mut child, action)?;
            Some(thread::spawn(move || {
                std::io::copy(&mut input, &mut child_stdin)?;
                child_stdin.flush()
            }))
        }
    };
    Ok((child, stdin_writer))
}

fn take_stdin(child: &mut ChildGuard, action: &str) -> Result<ChildStdin, ProcessError> {
    child
        .stdin
        .take()
        .ok_or_else(|| ProcessError::PipeUnavailable {
            action: action.to_string(),
            pipe: "stdin",
        })
}

fn wait_for_output(
    mut child: ChildGuard,
    stdin_writer: Option<StdinWriter>,
    limits: ProcessLimits,
    action: &str,
    cancellation: Option<&ProcessCancellation>,
) -> Result<Output, ProcessError> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ProcessError::PipeUnavailable {
            action: action.to_string(),
            pipe: "stdout",
        })?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ProcessError::PipeUnavailable {
            action: action.to_string(),
            pipe: "stderr",
        })?;
    let (stdout_limit_sender, stdout_limit_receiver) = mpsc::channel();
    let activity = StdoutActivity::start();
    let stdout = activity.reader(stdout);
    let stdout_reader =
        thread::spawn(move || read_stdout(stdout, limits.max_stdout_bytes, stdout_limit_sender));
    let stderr_reader =
        thread::spawn(move || read_stderr_diagnostic(stderr, STDERR_DIAGNOSTIC_BYTES));

    let mut status = None;
    let status = loop {
        if cancellation.is_some_and(ProcessCancellation::is_cancelled) {
            child.terminate_and_reap();
            join_writer(stdin_writer, action).ok();
            join_reader(stdout_reader, action).ok();
            join_reader(stderr_reader, action).ok();
            return Err(ProcessError::Cancelled {
                action: action.to_string(),
            });
        }
        if let Ok(max_stdout_bytes) = stdout_limit_receiver.try_recv() {
            child.terminate_and_reap();
            join_writer(stdin_writer, action).ok();
            join_reader(stdout_reader, action).ok();
            let stderr = join_reader(stderr_reader, action).unwrap_or_default();
            return Err(ProcessError::StdoutLimitExceeded {
                action: action.to_string(),
                max_stdout_bytes,
                diagnostic: diagnostic_suffix(&stderr, STDERR_DIAGNOSTIC_BYTES),
            });
        }
        if status.is_none() {
            status = child.try_wait().map_err(|source| ProcessError::Io {
                action: action.to_string(),
                source,
            })?;
        }
        let stdin_done = stdin_writer
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished);
        if stdout_reader.is_finished()
            && stderr_reader.is_finished()
            && stdin_done
            && let Some(status) = status.take()
        {
            break status;
        }
        if let Some(expiry) = limits.expiry(&activity) {
            child.terminate_and_reap();
            join_writer(stdin_writer, action).ok();
            join_reader(stdout_reader, action).ok();
            let stderr = join_reader(stderr_reader, action).unwrap_or_default();
            return Err(expiry.into_error(action, &stderr));
        }
        thread::sleep(limits.poll_interval(&activity));
    };

    child.disarm();
    join_writer(stdin_writer, action)?;
    let stdout = join_reader(stdout_reader, action)?;
    let stderr = join_reader(stderr_reader, action)?;
    if let Some(max_stdout_bytes) = limits.max_stdout_bytes
        && stdout.len() > max_stdout_bytes
    {
        return Err(ProcessError::StdoutLimitExceeded {
            action: action.to_string(),
            max_stdout_bytes,
            diagnostic: diagnostic_suffix(&stderr, STDERR_DIAGNOSTIC_BYTES),
        });
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}
