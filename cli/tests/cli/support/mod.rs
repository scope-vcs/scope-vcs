mod command;
#[path = "../../../src/test_support/git.rs"]
mod git;
mod git_output;
#[path = "../../../src/test_support/responses.rs"]
mod responses;
#[path = "../../../src/test_support/temp_dir.rs"]
mod temp_dir;

pub(crate) use command::scope_command;
pub(crate) use git::{commit_all, run_git};
pub(crate) use git_output::{assert_success, git_stdout};
pub(crate) use responses::repository_response;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
pub(crate) use temp_dir::TempDir;

pub(crate) fn scope_failure<const N: usize>(cwd: &Path, args: [&str; N], expected: &str) -> String {
    let output = scope_command(cwd).args(args).output().unwrap();
    assert_failure(&output, "scope command");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains(expected), "{stderr}");
    assert!(!stderr.contains("start browser login"), "{stderr}");
    stderr
}

pub(crate) fn create_repo_with_head(cwd: &Path) {
    run_git(cwd, ["-c", "init.defaultBranch=main", "init"]);
    fs::write(cwd.join("README.md"), "initial\n").unwrap();
    commit_all(cwd, "initial");
}

pub(crate) fn assert_failure(output: &Output, action: &str) {
    assert!(
        !output.status.success(),
        "{action} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) struct TestServer {
    pub api_url: String,
    pub config: TempDir,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TestServer {
    pub(crate) fn new(router: axum::Router) -> Self {
        Self::with_url(|_| router)
    }

    pub(crate) fn with_url(router: impl FnOnce(&str) -> axum::Router) -> Self {
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
                            stopped.await.ok();
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

    pub(crate) fn command(&self, cwd: &Path) -> Command {
        let mut command = scope_command(cwd);
        command
            .env("SCOPE_API_URL", &self.api_url)
            .env("XDG_CONFIG_HOME", self.config.path());
        command
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.take().unwrap().send(()).ok();
        let result = self.thread.take().unwrap().join();
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

pub(crate) fn session_response(id: &str, handle: &str, email: &str) -> serde_json::Value {
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
