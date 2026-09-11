mod support;

use axum::{
    Json, Router,
    http::StatusCode,
    routing::{delete, get, post},
};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use support::*;

const REMOTE_URL: &str = "https://scope.example/git/adam/sample";

#[test]
fn init_configures_an_unborn_repository_for_its_first_push() {
    let dir = TempDir::new("unborn");
    run_git(dir.path(), ["-c", "init.defaultBranch=main", "init"]);
    fs::create_dir(dir.path().join(".codex")).unwrap();
    let server = InitServer::start();

    let output = server
        .server
        .command(dir.path())
        .args(["init", "--name", "sample"])
        .output()
        .unwrap();
    server.finish();

    assert_success(&output, "scope init in unborn repository");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains(
            "Create your first commit including the generated Scope files, then run: scope push"
        ),
        "{stdout}"
    );
    assert!(!stderr.contains("No such remote"), "{stderr}");
    assert!(dir.path().join(".scope/RULES.md").is_file());
    assert!(
        fs::read_to_string(dir.path().join("AGENTS.md"))
            .unwrap()
            .contains("Read and follow `.scope/RULES.md`")
    );
    assert!(dir.path().join(".git/scope/repo.json").is_file());
    assert_eq!(
        git_stdout(dir.path(), ["remote", "get-url", "scope"]),
        REMOTE_URL
    );
    assert!(
        !std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn init_warns_on_dirty_working_tree_and_continues_to_auth() {
    let dir = TempDir::new("dirty");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("README.md"), "uncommitted\n").unwrap();

    let output = scope_command(dir.path())
        .args(["init", "--name", "sample"])
        .output()
        .unwrap();

    assert_failure(&output, "scope init with dirty working tree");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Working tree has uncommitted changes."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Only committed HEAD will be pushed to Scope."),
        "{stderr}"
    );
    assert!(stderr.contains("scope login"), "{stderr}");
}

#[test]
fn init_json_is_one_complete_result() {
    let dir = TempDir::new("init-json");
    create_repo_with_head(dir.path());
    let server = InitServer::start();
    let output = server
        .server
        .command(dir.path())
        .args(["--json", "init", "--name", "sample"])
        .output()
        .unwrap();
    server.finish();
    assert_success(&output, "scope init --json");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["command"], "init");
    assert_eq!(value["result"]["repository"], "adam/sample");
    assert_eq!(value["result"]["remote"], "scope");
}

#[test]
fn init_requires_explicit_name_when_noninteractive() {
    let dir = TempDir::new("init-no-name");
    create_repo_with_head(dir.path());
    let output = scope_command(dir.path())
        .args(["--json", "init"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert!(value["message"].as_str().unwrap().contains("--name"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Repository name ["));
}

#[test]
fn init_partial_failure_json_identifies_retained_repository() {
    let dir = TempDir::new("init-partial-json");
    create_repo_with_head(dir.path());
    run_git(
        dir.path(),
        ["remote", "add", "scope", "https://old.scope.example/repo"],
    );
    fs::write(dir.path().join(".git/scope"), "block state directory").unwrap();
    let server = InitServer::start();
    let output = server
        .server
        .command(dir.path())
        .args(["--json", "init", "--name", "sample"])
        .output()
        .unwrap();
    assert!(!server.finish());
    assert_eq!(
        git_stdout(dir.path(), ["remote", "get-url", "scope"]),
        "https://old.scope.example/repo"
    );
    assert_eq!(output.status.code(), Some(5));
    let value: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(value["recovery"]["repository"], "adam/sample");
    assert_eq!(value["recovery"]["created"], true);
    assert_eq!(value["recovery"]["configured"], false);
    assert!(
        value["recovery"]["recovery_commands"]
            .as_array()
            .unwrap()
            .len()
            >= 3
    );
}

struct InitServer {
    server: TestServer,
    rolled_back: Arc<AtomicBool>,
}

impl InitServer {
    fn start() -> Self {
        let rolled_back = Arc::new(AtomicBool::new(false));
        let rollback_state = rolled_back.clone();
        let app = Router::new()
            .route("/v1/session", get(|| async { Json(session_response()) }))
            .route("/v1/repos", post(|| async { Json(create_repo_response()) }))
            .route(
                "/v1/repos/adam/sample",
                delete(move || {
                    let rollback_state = rollback_state.clone();
                    async move {
                        rollback_state.store(true, Ordering::SeqCst);
                        StatusCode::NO_CONTENT
                    }
                }),
            );
        Self {
            server: TestServer::new(app),
            rolled_back,
        }
    }

    fn finish(self) -> bool {
        drop(self.server);
        self.rolled_back.load(Ordering::SeqCst)
    }
}

fn session_response() -> serde_json::Value {
    serde_json::json!({
        "identity": null,
        "user": {
            "id": "user_test",
            "handle": "adam",
            "email": "adam@example.test",
            "email_verified": true
        }
    })
}

fn create_repo_response() -> serde_json::Value {
    let repo = serde_json::json!({
        "id": "repo_test",
        "owner_handle": "adam",
        "name": "sample",
        "git_remote_url": REMOTE_URL,
        "lifecycle_state": "AwaitingFirstPush",
        "change_version": 1,
        "access": {
            "actor": "Owner",
            "can_read_private_files": true,
            "can_push": true,
            "can_change_file_visibility": true,
            "can_manage_members": true,
            "can_delete_repo": true
        },
        "open_request_count": 0,
        "request_permissions": { "can_start_request": true }
    });
    serde_json::json!({
        "repo": repo,
        "init": {
            "repo": repo,
            "git_remote_url": REMOTE_URL,
            "remote_name": "scope",
            "push_branch": "main",
            "token": null,
            "push_token": null
        }
    })
}
