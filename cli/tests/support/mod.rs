use std::{
    fs,
    path::Path,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

#[allow(dead_code)]
#[path = "../../src/test_support.rs"]
mod test_support;
#[allow(unused_imports)]
pub(crate) use test_support::{TempDir, commit_all, repository_response, run_git};

pub fn scope_command(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scope"));
    command.current_dir(cwd);
    command.env(
        "SCOPE_API_URL",
        format!("http://127.0.0.1:9/scope-cli-test-{}", unix_nanos()),
    );
    command
}

#[allow(dead_code)]
pub fn scope_failure<const N: usize>(cwd: &Path, args: [&str; N], expected: &str) -> String {
    let output = scope_command(cwd).args(args).output().unwrap();
    assert_failure(&output, "scope command");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(expected), "{stderr}");
    assert!(!stderr.contains("start browser login"), "{stderr}");
    stderr
}

#[allow(dead_code)]
pub fn scope_failure_with_code<const N: usize>(
    cwd: &Path,
    args: [&str; N],
    expected: &str,
    code: i32,
) -> String {
    let output = scope_command(cwd).args(args).output().unwrap();
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(expected), "{stderr}");
    stderr
}
#[allow(dead_code)]
pub fn create_repo_with_head(cwd: &Path) {
    run_git(cwd, ["-c", "init.defaultBranch=main", "init"]);
    fs::write(cwd.join("README.md"), "initial\n").unwrap();
    commit_all(cwd, "initial");
}

pub fn assert_failure(output: &Output, action: &str) {
    assert!(
        !output.status.success(),
        "{action} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[allow(dead_code)]
pub struct TestServer {
    pub api_url: String,
    pub config: TempDir,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[allow(dead_code)]
impl TestServer {
    pub fn new(router: axum::Router) -> Self {
        Self::with_url(|_| router)
    }

    pub fn with_url(router: impl FnOnce(&str) -> axum::Router) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let config = TempDir::new("server-config");
        let sessions = config.path().join("scope/sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::write(
            sessions.join(scope_cli::auth::session_storage_key(&api_url)),
            "test-token",
        )
        .unwrap();
        let router = router(&api_url);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async move {
                    axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), router)
                        .with_graceful_shutdown(async {
                            let _ = stopped.await;
                        })
                        .await
                        .unwrap();
                });
        });
        Self {
            api_url,
            config,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    pub fn command(&self, cwd: &Path) -> Command {
        let mut command = scope_command(cwd);
        command
            .env("SCOPE_API_URL", &self.api_url)
            .env("XDG_CONFIG_HOME", self.config.path());
        command
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.stop.take().unwrap().send(());
        let result = self.thread.take().unwrap().join();
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

#[allow(dead_code)]
pub fn assert_success(output: &Output, action: &str) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[allow(dead_code)]
pub fn git_stdout(
    cwd: &Path,
    args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
) -> String {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert_success(&output, "inspect Git checkout");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[allow(dead_code)]
pub fn session_response(id: &str, handle: &str, email: &str) -> serde_json::Value {
    serde_json::to_value(scope_api_contract::AccountSessionResponse {
        identity: None,
        user: Some(scope_api_contract::UserResponse {
            id: id.into(),
            handle: handle.into(),
            email: email.into(),
            email_verified: true,
        }),
    })
    .unwrap()
}
