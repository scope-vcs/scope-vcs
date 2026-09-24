mod support;

use axum::{Json, Router, routing::get};
use scope_cli::repo_config::{
    load_worktree_scope_repo_config, load_worktree_scope_repo_config_base_hash, repo_config_path,
};
use serde_json::{Value, json};
use std::fs;
use support::*;

#[test]
fn failed_fetch_reports_visibility_setup_and_retry_keeps_local_state() {
    let dir = TempDir::new("pull-visibility-recovery");
    create_repo_with_head(dir.path());
    let server = TestServer::new(
        Router::new()
            .route(
                "/v1/session",
                get(|| async {
                    Json(session_response(
                        "user_test",
                        "reader",
                        "reader@example.test",
                    ))
                }),
            )
            .route(
                "/v1/repos/owner/repo",
                get(|| async { Json(repository_response(json!({}))) }),
            ),
    );
    run_git(
        dir.path(),
        [
            "remote",
            "add",
            "scope",
            &format!("{}/git/public/owner/repo", server.api_url),
        ],
    );
    let original_head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let config_path = repo_config_path(dir.path()).unwrap();

    let first = failed_pull(&server, dir.path());
    assert_eq!(first["recovery"]["visibility_setup"], "created");
    assert_eq!(first["recovery"]["operation"], "pull");
    assert!(
        first["recovery"]["recovery"]
            .as_str()
            .unwrap()
            .contains("rerun scope pull")
    );
    let config = load_worktree_scope_repo_config(dir.path()).unwrap();
    let original_base = load_worktree_scope_repo_config_base_hash(dir.path()).unwrap();
    assert_eq!(git_stdout(dir.path(), ["rev-parse", "HEAD"]), original_head);

    fs::remove_file(config_path.parent().unwrap().join("repo-state.json")).unwrap();
    let recovered = failed_pull(&server, dir.path());
    assert_eq!(recovered["recovery"]["visibility_setup"], "base_recovered");
    assert_eq!(recovered["error"]["code"], first["error"]["code"]);
    assert_eq!(load_worktree_scope_repo_config(dir.path()).unwrap(), config);
    assert_eq!(
        load_worktree_scope_repo_config_base_hash(dir.path()).unwrap(),
        original_base
    );

    let retry = failed_pull(&server, dir.path());
    assert!(retry["recovery"].is_null());
    assert_eq!(retry["error"]["code"], first["error"]["code"]);
    assert_eq!(git_stdout(dir.path(), ["rev-parse", "HEAD"]), original_head);
}

fn failed_pull(server: &TestServer, cwd: &std::path::Path) -> Value {
    let output = server
        .command(cwd)
        .args(["--json", "pull"])
        .output()
        .unwrap();
    assert_failure(&output, "scope pull with unavailable Git endpoint");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    serde_json::from_str(stderr.lines().last().unwrap()).unwrap()
}
