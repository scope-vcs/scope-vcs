mod support;

use scope_cli::clone::parse_repo_spec;

#[test]
fn parse_repo_spec_rejects_urls_and_partial_specs() {
    for repository in [
        "",
        "adam",
        "adam/",
        "/scope-vcs",
        "adam/scope-vcs/extra",
        "https://scopevcs.com/git/adam/scope-vcs",
    ] {
        assert!(parse_repo_spec(repository).is_err(), "{repository}");
    }
}

#[test]
fn clone_without_auth_returns_authentication_json() {
    let dir = support::TempDir::new("clone-auth");
    let output = support::scope_command(dir.path())
        .args(["--json", "clone", "adam/repo"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let value: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(value["code"], "unauthorized");
    assert!(value["message"].as_str().unwrap().contains("scope login"));
}

#[test]
fn clone_json_keeps_git_output_off_stdout() {
    use axum::{Json, Router, routing::get};
    let source = support::TempDir::new("clone-json-source");
    support::create_repo_with_head(source.path());
    let destination = support::TempDir::new("clone-json-destination");
    let checkout = destination.path().join("checkout");
    let remote_url = format!("file://{}", source.path().display());
    let response = serde_json::json!({
        "id": "repo_test", "owner_handle": "adam", "name": "sample", "git_remote_url": remote_url,
        "lifecycle_state": "Ready", "change_version": 1, "open_request_count": 0,
        "access": {"actor": "Public", "can_read_private_files": false, "can_push": false,
            "can_change_file_visibility": false, "can_manage_members": false, "can_delete_repo": false},
        "request_permissions": {"can_start_request": true}
    });
    let app = Router::new().route(
        "/v1/repos/adam/sample",
        get(move || {
            let response = response.clone();
            async { Json(response) }
        }),
    );
    let server = support::TestServer::new(app);
    let output = server
        .command(destination.path())
        .args(["--json", "clone", "adam/sample"])
        .arg(&checkout)
        .output()
        .unwrap();
    drop(server);
    assert!(
        output.status.success(),
        "scope clone --json: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["command"], "clone");
    assert_eq!(value["result"]["repository"], "adam/sample");
    assert_eq!(value["result"]["configured"], true);
    assert!(checkout.join("README.md").is_file());
    assert!(
        scope_cli::repo_config::repo_config_path(&checkout)
            .unwrap()
            .is_file()
    );
    assert_eq!(
        support::git_stdout(
            &checkout,
            [
                "config",
                "--local",
                "--get-urlmatch",
                "credential.helper",
                &remote_url
            ]
        ),
        "!scope git-credential"
    );
    assert_eq!(
        scope_cli::repo_config::load_worktree_scope_repo_config(&checkout).unwrap(),
        scope_cli::repo_config::default_scope_repo_config()
    );
}
