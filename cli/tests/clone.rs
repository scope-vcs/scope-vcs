mod support;

use scope_cli::clone::{RepoSpec, parse_repo_spec};

#[test]
fn parse_repo_spec_accepts_owner_and_repo() {
    assert_eq!(
        parse_repo_spec(" adam/scope-vcs ").unwrap(),
        RepoSpec {
            owner: "adam".to_string(),
            repo: "scope-vcs".to_string(),
        }
    );
}

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
    use std::{fs, net::TcpListener, thread};
    let source = support::TempDir::new("clone-json-source");
    support::create_repo_with_head(source.path());
    let destination = support::TempDir::new("clone-json-destination");
    let config = support::TempDir::new("clone-json-auth");
    let checkout = destination.path().join("checkout");
    let remote_url = format!("file://{}", source.path().display());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let response = serde_json::json!({
        "id": "repo_test", "owner_handle": "adam", "name": "sample", "git_remote_url": remote_url,
        "lifecycle_state": "Ready", "change_version": 1, "open_request_count": 0,
        "access": {"actor": "Public", "can_read_private_files": false, "can_push": false,
            "can_change_file_visibility": false, "can_apply_changes": false, "can_manage_members": false, "can_delete_repo": false},
        "request_permissions": {"can_start_request": true}
    });
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                let app = Router::new().route(
                    "/v1/repos/adam/sample",
                    get(move || {
                        let response = response.clone();
                        async { Json(response) }
                    }),
                );
                axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app)
                    .with_graceful_shutdown(async {
                        let _ = stopped.await;
                    })
                    .await
                    .unwrap();
            });
    });
    let sessions = config.path().join("scope/sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join(format!("cli-session-{}", hex::encode(api_url.as_bytes()))),
        "test-token",
    )
    .unwrap();
    let output = support::scope_command(destination.path())
        .env("SCOPE_API_URL", &api_url)
        .env("XDG_CONFIG_HOME", config.path())
        .args(["--json", "clone", "adam/sample"])
        .arg(&checkout)
        .output()
        .unwrap();
    let _ = stop.send(());
    server.join().unwrap();
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
}
