use crate::ProcessError;
use std::{
    io::Read,
    sync::{Arc, Mutex, PoisonError, mpsc},
    thread,
    time::{Duration, Instant},
};

pub const STDERR_DIAGNOSTIC_BYTES: usize = 8 * 1024;

#[derive(Clone)]
pub(crate) struct StdoutActivity {
    started_at: Instant,
    awaiting_output_since: Arc<Mutex<Option<Instant>>>,
}

impl StdoutActivity {
    pub(crate) fn start() -> Self {
        let started_at = Instant::now();
        Self {
            started_at,
            awaiting_output_since: Arc::new(Mutex::new(Some(started_at))),
        }
    }

    pub(crate) fn reader<R: Read>(&self, stdout: R) -> StdoutActivityReader<R> {
        StdoutActivityReader {
            stdout,
            activity: self.clone(),
        }
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub(crate) fn idle(&self) -> Duration {
        self.awaiting_output_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .map_or(Duration::ZERO, |since| since.elapsed())
    }

    fn record(&self, awaiting_output_since: Option<Instant>) {
        *self
            .awaiting_output_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = awaiting_output_since;
    }
}

pub(crate) struct StdoutActivityReader<R> {
    stdout: R,
    activity: StdoutActivity,
}

impl<R: Read> Read for StdoutActivityReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.stdout.read(buffer)?;
        self.activity.record((read > 0).then(Instant::now));
        Ok(read)
    }
}

pub(crate) fn read_stdout(
    mut stdout: impl Read,
    max_bytes: Option<usize>,
    limit_sender: mpsc::Sender<usize>,
) -> std::io::Result<Vec<u8>> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    let mut limit_reported = false;
    loop {
        let read = stdout.read(&mut buffer)?;
        if read == 0 {
            return Ok(retained);
        }
        match max_bytes {
            Some(max_bytes) => {
                let max_retained = max_bytes.saturating_add(1);
                let remaining = max_retained.saturating_sub(retained.len());
                if remaining > 0 {
                    retained.extend_from_slice(&buffer[..read.min(remaining)]);
                }
                if retained.len() > max_bytes && !limit_reported {
                    limit_sender.send(max_bytes).ok();
                    limit_reported = true;
                }
            }
            None => retained.extend_from_slice(&buffer[..read]),
        }
    }
}

pub(crate) fn read_stderr_diagnostic(
    mut stderr: impl Read,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    let max_retained = max_bytes.saturating_add(1);
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = stderr.read(&mut buffer)?;
        if read == 0 {
            return Ok(retained);
        }
        let remaining = max_retained.saturating_sub(retained.len());
        if remaining > 0 {
            retained.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
}

pub(crate) fn join_reader(
    handle: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    action: &str,
) -> Result<Vec<u8>, ProcessError> {
    handle
        .join()
        .map_err(|_| ProcessError::ThreadPanicked {
            action: action.to_string(),
        })?
        .map_err(|source| ProcessError::Io {
            action: action.to_string(),
            source,
        })
}

pub(crate) fn join_writer(
    handle: Option<thread::JoinHandle<std::io::Result<()>>>,
    action: &str,
) -> Result<(), ProcessError> {
    let Some(handle) = handle else {
        return Ok(());
    };
    match handle.join().map_err(|_| ProcessError::ThreadPanicked {
        action: action.to_string(),
    })? {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(source) => Err(ProcessError::Io {
            action: action.to_string(),
            source,
        }),
    }
}

pub(crate) fn diagnostic_suffix(stderr: &[u8], max_bytes: usize) -> String {
    let message = truncated_stderr(stderr, max_bytes);
    if message.is_empty() {
        String::new()
    } else {
        format!(": {message}")
    }
}

pub fn truncated_stderr(stderr: &[u8], max_bytes: usize) -> String {
    let mut message = String::from_utf8_lossy(stderr).trim().to_string();
    if message.len() > max_bytes {
        let mut end = 0;
        for (index, character) in message.char_indices() {
            let next = index + character.len_utf8();
            if next > max_bytes {
                break;
            }
            end = next;
        }
        message.truncate(end);
        message.push_str("...");
    }
    message
}
