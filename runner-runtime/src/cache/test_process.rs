use std::process::{Command, Stdio};

pub(super) fn isolated(scenario: impl FnOnce()) {
    let thread = std::thread::current();
    isolated_with(thread.name().unwrap(), scenario, || {});
}

fn isolated_with(name: &str, scenario: impl FnOnce(), concurrent: impl FnOnce()) {
    if std::env::var("SCOPE_CACHE_TEST").as_deref() == Ok(name) {
        scenario();
        return;
    }
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env("SCOPE_CACHE_TEST", name)
        .env("SCOPE_CACHE_TEST_PARENT", std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    concurrent();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{name}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
#[expect(unsafe_code)]
mod tests {
    use super::*;
    use crate::cache::archive::{create_archive, extract_reader};
    use std::{
        fs,
        io::{Read, Write},
        os::{
            fd::AsRawFd,
            unix::net::{UnixListener, UnixStream},
        },
        path::{Path, PathBuf},
    };

    struct PausedExecutable<R> {
        reader: R,
        executable: PathBuf,
        controller: UnixStream,
        paused: bool,
    }

    impl<R: Read> Read for PausedExecutable<R> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.paused && executable_is_open_for_writing(&self.executable) {
                self.paused = true;
                self.controller.write_all(&[1])?;
                self.controller.read_exact(&mut [0])?;
            }
            self.reader.read(buffer)
        }
    }

    fn executable_is_open_for_writing(executable: &Path) -> bool {
        fs::read_dir("/proc/self/fd").unwrap().any(|entry| {
            let entry = entry.unwrap();
            if fs::read_link(entry.path()).ok().as_deref() != Some(executable) {
                return false;
            }
            let fd = entry.file_name().to_str().unwrap().parse::<i32>().unwrap();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            flags >= 0 && flags & libc::O_ACCMODE != libc::O_RDONLY
        })
    }

    struct PendingExec {
        pid: libc::pid_t,
        release: Option<UnixStream>,
    }

    impl PendingExec {
        fn fork() -> Self {
            let (release, waiting) = UnixStream::pair().unwrap();
            let waiting_fd = waiting.as_raw_fd();
            let pid = unsafe { libc::fork() };
            assert!(pid >= 0, "{}", std::io::Error::last_os_error());
            if pid == 0 {
                unsafe {
                    libc::close(release.as_raw_fd());
                    let mut byte = 0_u8;
                    while libc::read(waiting_fd, (&raw mut byte).cast(), 1) == -1 {
                        if *libc::__errno_location() != libc::EINTR {
                            break;
                        }
                    }
                    libc::_exit(0);
                }
            }
            Self {
                pid,
                release: Some(release),
            }
        }
    }

    impl Drop for PendingExec {
        fn drop(&mut self) {
            self.release.take();
            let mut status = 0;
            while unsafe { libc::waitpid(self.pid, &raw mut status, 0) } == -1 {
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    break;
                }
            }
        }
    }

    #[test]
    fn restored_executable_does_not_inherit_a_sibling_tests_pending_exec() {
        let thread = std::thread::current();
        let name = thread.name().unwrap();
        let parent = std::env::var("SCOPE_CACHE_TEST_PARENT")
            .unwrap_or_else(|_| std::process::id().to_string());
        let socket = std::env::temp_dir().join(format!("scope-cache-{parent}.sock"));
        let listener = if std::env::var("SCOPE_CACHE_TEST").as_deref() == Ok(name) {
            None
        } else {
            Some(UnixListener::bind(&socket).unwrap())
        };
        isolated_with(
            name,
            || {
                let source = tempfile::tempdir().unwrap();
                let restored = tempfile::tempdir().unwrap();
                let archive = tempfile::NamedTempFile::new().unwrap();
                fs::copy("/bin/true", source.path().join("build_script_build")).unwrap();
                create_archive(source.path(), archive.path(), None).unwrap();
                let executable = restored
                    .path()
                    .canonicalize()
                    .unwrap()
                    .join("build_script_build");
                let mut controller = UnixStream::connect(&socket).unwrap();
                let path = executable.as_os_str().as_encoded_bytes();
                controller
                    .write_all(&(path.len() as u64).to_be_bytes())
                    .unwrap();
                controller.write_all(path).unwrap();
                let decoder = zstd::Decoder::new(fs::File::open(archive.path()).unwrap()).unwrap();
                let reader = PausedExecutable {
                    reader: decoder,
                    executable,
                    controller: controller.try_clone().unwrap(),
                    paused: false,
                };
                extract_reader(reader, restored.path(), None).unwrap();
                controller.write_all(&[2]).unwrap();
                controller.read_exact(&mut [0]).unwrap();
            },
            || {
                let (mut controller, _) = listener.as_ref().unwrap().accept().unwrap();
                let mut length = [0; 8];
                controller.read_exact(&mut length).unwrap();
                let mut path = vec![0; u64::from_be_bytes(length) as usize];
                controller.read_exact(&mut path).unwrap();
                let executable = PathBuf::from(String::from_utf8(path).unwrap());
                let mut event = [0];
                controller.read_exact(&mut event).unwrap();
                assert_eq!(event, [1]);
                let pending = PendingExec::fork();
                controller.write_all(&[1]).unwrap();
                controller.read_exact(&mut event).unwrap();
                assert_eq!(event, [2]);
                let execution = Command::new(&executable).status();
                drop(pending);
                controller.write_all(&[1]).unwrap();
                fs::remove_file(&socket).unwrap();
                assert!(
                    execution.as_ref().is_ok_and(|status| status.success()),
                    "{execution:?}"
                );
            },
        );
    }
}
