use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

#[derive(Clone, Debug)]
pub struct CodecProcessLimits {
    pub timeout: Duration,
    pub memory_bytes: u64,
    pub output_file_bytes: u64,
    pub captured_output_bytes: usize,
    pub cpu_seconds: u64,
    pub open_files: u64,
    pub processes: u64,
}

#[derive(Debug)]
pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessFailure {
    #[error("could not start {program}: {source}")]
    Start { program: String, source: io::Error },
    #[error("{program} exceeded its {seconds}-second time limit")]
    Timeout { program: String, seconds: u64 },
    #[error("{program} failed with {status}: {stderr}")]
    Exit {
        program: String,
        status: String,
        stderr: String,
    },
    #[error("could not collect {program} output: {source}")]
    Output { program: String, source: io::Error },
}

pub async fn run_bounded(
    program: &Path,
    args: &[OsString],
    cwd: &Path,
    limits: &CodecProcessLimits,
) -> Result<ProcessOutput, ProcessFailure> {
    let program_label = program.display().to_string();
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env(
            "PATH",
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        )
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("TMPDIR", cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    install_process_limits(&mut command, limits.clone());

    let mut child = command.spawn().map_err(|source| ProcessFailure::Start {
        program: program_label.clone(),
        source,
    })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let capture_limit = limits.captured_output_bytes;
    let stdout_task = tokio::spawn(read_capped(stdout, capture_limit));
    let stderr_task = tokio::spawn(read_capped(stderr, capture_limit));

    let status = match tokio::time::timeout(limits.timeout, child.wait()).await {
        Ok(status) => status.map_err(|source| ProcessFailure::Output {
            program: program_label.clone(),
            source,
        })?,
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(ProcessFailure::Timeout {
                program: program_label,
                seconds: limits.timeout.as_secs(),
            });
        }
    };
    let stdout = join_output(stdout_task, &program_label).await?;
    let stderr = join_output(stderr_task, &program_label).await?;
    if !status.success() {
        return Err(ProcessFailure::Exit {
            program: program_label,
            status: status.to_string(),
            stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
        });
    }
    Ok(ProcessOutput { stdout, stderr })
}

async fn read_capped<R>(mut reader: R, retained_limit: usize) -> io::Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut retained = Vec::with_capacity(retained_limit.min(64 * 1024));
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            return Ok(retained);
        }
        let remaining = retained_limit.saturating_sub(retained.len());
        retained.extend_from_slice(&chunk[..read.min(remaining)]);
    }
}

async fn join_output(
    task: tokio::task::JoinHandle<io::Result<Vec<u8>>>,
    program: &str,
) -> Result<Vec<u8>, ProcessFailure> {
    match task.await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(source)) => Err(ProcessFailure::Output {
            program: program.to_owned(),
            source,
        }),
        Err(error) => Err(ProcessFailure::Output {
            program: program.to_owned(),
            source: io::Error::other(error.to_string()),
        }),
    }
}

fn install_process_limits(command: &mut Command, limits: CodecProcessLimits) {
    // SAFETY: this closure only invokes async-signal-safe setrlimit calls before exec.
    unsafe {
        command.pre_exec(move || {
            set_limit(libc::RLIMIT_CORE, 0)?;
            set_limit(libc::RLIMIT_AS, limits.memory_bytes)?;
            set_limit(libc::RLIMIT_FSIZE, limits.output_file_bytes)?;
            set_limit(libc::RLIMIT_CPU, limits.cpu_seconds)?;
            set_limit(libc::RLIMIT_NOFILE, limits.open_files)?;
            set_limit(libc::RLIMIT_NPROC, limits.processes)?;
            Ok(())
        });
    }
}

fn set_limit(resource: libc::__rlimit_resource_t, value: u64) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value,
        rlim_max: value,
    };
    // SAFETY: limit points to a valid rlimit value for the duration of the call.
    if unsafe { libc::setrlimit(resource, &limit) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn os_args<const N: usize>(args: [&str; N]) -> Vec<OsString> {
    args.into_iter().map(OsString::from).collect()
}

pub fn path_arg(path: impl Into<PathBuf>) -> OsString {
    path.into().into_os_string()
}
