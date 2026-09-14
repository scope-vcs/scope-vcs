use super::{CancellationToken, Cancelled};
use anyhow::{Context, bail};
use std::{
    io::{self, Read, Write},
    process::{Child, Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_STDERR_BYTES: usize = 1024 * 1024;

pub fn run_cancellable(
    command: &mut Command,
    input: Option<Vec<u8>>,
    token: &CancellationToken,
    timeout: Duration,
    max_stdout_bytes: usize,
) -> anyhow::Result<Output> {
    token.check()?;
    configure_process_group(command);
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("start child process")?;
    let process_tree = match ProcessTree::attach(&child) {
        Ok(process_tree) => process_tree,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error).context("manage child process tree");
        }
    };
    let stdout = child.stdout.take().context("capture child stdout")?;
    let stderr = child.stderr.take().context("capture child stderr")?;
    let stdout_exceeded = Arc::new(AtomicBool::new(false));
    let stdout_reader = spawn_bounded_reader(stdout, max_stdout_bytes, &stdout_exceeded);
    let stderr_exceeded = Arc::new(AtomicBool::new(false));
    let stderr_reader = spawn_bounded_reader(stderr, MAX_STDERR_BYTES, &stderr_exceeded);
    let stdin_writer = input.map(|input| {
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = Arc::clone(&done);
        let mut stdin = child.stdin.take().expect("piped stdin was requested");
        let handle = thread::spawn(move || {
            let result = stdin.write_all(&input);
            drop(stdin);
            worker_done.store(true, Ordering::Release);
            result
        });
        WriteWorker { handle, done }
    });

    let started = Instant::now();
    let mut exit_status = None;
    let status = loop {
        if token.is_cancelled() {
            terminate_and_reap(&mut child, &process_tree);
            join_io(stdout_reader, stderr_reader, stdin_writer)?;
            return Err(Cancelled.into());
        }
        if started.elapsed() >= timeout {
            terminate_and_reap(&mut child, &process_tree);
            join_io(stdout_reader, stderr_reader, stdin_writer)?;
            bail!("child process timed out after {}s", timeout.as_secs_f64());
        }
        if stdout_exceeded.load(Ordering::Acquire) {
            terminate_and_reap(&mut child, &process_tree);
            join_io(stdout_reader, stderr_reader, stdin_writer)?;
            bail!("child process stdout exceeded {max_stdout_bytes} bytes");
        }
        if stderr_exceeded.load(Ordering::Acquire) {
            terminate_and_reap(&mut child, &process_tree);
            join_io(stdout_reader, stderr_reader, stdin_writer)?;
            bail!("child process stderr exceeded {MAX_STDERR_BYTES} bytes");
        }
        if exit_status.is_none() {
            match child.try_wait() {
                Ok(status) => exit_status = status,
                Err(error) => {
                    terminate_and_reap(&mut child, &process_tree);
                    join_io(stdout_reader, stderr_reader, stdin_writer)?;
                    return Err(error).context("wait for child process");
                }
            }
        }
        let stdin_done = stdin_writer
            .as_ref()
            .is_none_or(|writer| writer.done.load(Ordering::Acquire));
        if let Some(status) = exit_status
            && stdout_reader.done.load(Ordering::Acquire)
            && stderr_reader.done.load(Ordering::Acquire)
            && stdin_done
        {
            break status;
        }
        thread::sleep(POLL_INTERVAL);
    };

    let (stdout, stderr) = join_io(stdout_reader, stderr_reader, stdin_writer)?;
    if stdout_exceeded.load(Ordering::Acquire) {
        bail!("child process stdout exceeded {max_stdout_bytes} bytes");
    }
    if stderr_exceeded.load(Ordering::Acquire) {
        bail!("child process stderr exceeded {MAX_STDERR_BYTES} bytes");
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn spawn_bounded_reader(
    mut pipe: impl Read + Send + 'static,
    limit: usize,
    exceeded: &Arc<AtomicBool>,
) -> ReadWorker {
    let exceeded = Arc::clone(exceeded);
    let done = Arc::new(AtomicBool::new(false));
    let worker_done = Arc::clone(&done);
    let handle = thread::spawn(move || {
        let result = (|| {
            let mut output = Vec::with_capacity(limit.min(8192));
            let mut buffer = [0_u8; 8192];
            loop {
                let count = pipe.read(&mut buffer)?;
                if count == 0 {
                    return Ok(output);
                }
                let remaining = limit.saturating_sub(output.len());
                output.extend_from_slice(&buffer[..count.min(remaining)]);
                if count > remaining {
                    exceeded.store(true, Ordering::Release);
                }
            }
        })();
        worker_done.store(true, Ordering::Release);
        result
    });
    ReadWorker { handle, done }
}

struct ReadWorker {
    handle: thread::JoinHandle<io::Result<Vec<u8>>>,
    done: Arc<AtomicBool>,
}

struct WriteWorker {
    handle: thread::JoinHandle<io::Result<()>>,
    done: Arc<AtomicBool>,
}

fn join_io(
    stdout: ReadWorker,
    stderr: ReadWorker,
    stdin: Option<WriteWorker>,
) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    if let Some(stdin) = stdin {
        match stdin.handle.join() {
            Ok(Ok(())) | Ok(Err(_)) => {}
            Err(_) => bail!("child stdin writer panicked"),
        }
    }
    let stdout = stdout
        .handle
        .join()
        .map_err(|_| anyhow::anyhow!("child stdout reader panicked"))?
        .context("read child stdout")?;
    let stderr = stderr
        .handle
        .join()
        .map_err(|_| anyhow::anyhow!("child stderr reader panicked"))?
        .context("read child stderr")?;
    Ok((stdout, stderr))
}

fn terminate_and_reap(child: &mut Child, process_tree: &ProcessTree) {
    process_tree.terminate(child);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn configure_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
struct ProcessTree;

#[cfg(unix)]
impl ProcessTree {
    fn attach(_child: &Child) -> io::Result<Self> {
        Ok(Self)
    }

    fn terminate(&self, child: &Child) {
        if let Ok(process_group) = i32::try_from(child.id()) {
            // SAFETY: the child was spawned as the leader of this process group.
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
    }
}

#[cfg(windows)]
struct ProcessTree {
    job: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl ProcessTree {
    fn attach(child: &Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        // std::process::Command cannot expose a suspended process's primary thread. Assigning the
        // trusted Git/Node child immediately after spawn is the narrowest reliable std boundary.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        };
        let assigned = configured != 0
            && unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as _) } != 0;
        if !assigned {
            let error = io::Error::last_os_error();
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(job);
            }
            return Err(error);
        }
        Ok(Self { job })
    }

    fn terminate(&self, _child: &Child) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

#[cfg(not(any(unix, windows)))]
struct ProcessTree;

#[cfg(not(any(unix, windows)))]
impl ProcessTree {
    fn attach(_child: &Child) -> io::Result<Self> {
        Ok(Self)
    }

    fn terminate(&self, _child: &Child) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn cancellation_stops_and_reaps_a_child() {
        let token = CancellationToken::new();
        let cancel = token.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(40));
            cancel.cancel();
        });
        let started = Instant::now();
        let error = run_cancellable(
            Command::new("sh").args(["-c", "sleep 30"]),
            None,
            &token,
            Duration::from_secs(5),
            1024,
        )
        .unwrap_err();
        assert!(error.is::<Cancelled>());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn captures_input_and_bounded_output() {
        let output = run_cancellable(
            Command::new("sh").args(["-c", "read value; printf '%s' \"$value\""]),
            Some(b"fixture\n".to_vec()),
            &CancellationToken::new(),
            Duration::from_secs(2),
            1024,
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"fixture");

        let error = run_cancellable(
            Command::new("sh").args(["-c", "printf 12345"]),
            None,
            &CancellationToken::new(),
            Duration::from_secs(2),
            4,
        )
        .unwrap_err();
        assert!(error.to_string().contains("stdout exceeded 4 bytes"));
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_before_spawn_has_no_side_effect() {
        let token = CancellationToken::new();
        token.cancel();
        let error = run_cancellable(
            Command::new("sh").args(["-c", "exit 99"]),
            None,
            &token,
            Duration::from_secs(2),
            1024,
        )
        .unwrap_err();
        assert!(error.is::<Cancelled>());
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_descendants_that_keep_output_pipes_open() {
        let started = Instant::now();
        let error = run_cancellable(
            Command::new("sh").args(["-c", "sleep 30 &"]),
            None,
            &CancellationToken::new(),
            Duration::from_millis(100),
            1024,
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(windows)]
    #[test]
    fn timeout_kills_windows_descendants_after_the_leader_exits() {
        let started = Instant::now();
        let error = run_cancellable(
            Command::new("powershell.exe").args([
                "-NoProfile",
                "-Command",
                "Start-Process -NoNewWindow ping.exe -ArgumentList '-n','30','127.0.0.1'; exit",
            ]),
            None,
            &CancellationToken::new(),
            Duration::from_millis(500),
            1024,
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
