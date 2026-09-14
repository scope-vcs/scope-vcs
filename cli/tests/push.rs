mod support;

#[cfg(unix)]
use axum::{Json, Router, routing::get};
use scope_cli::repo_config::repo_config_path;
use std::fs;
#[cfg(unix)]
use std::{
    fs::File,
    io::Read,
    os::fd::FromRawFd,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use support::*;

#[test]
fn push_stops_at_repository_preconditions_before_login() {
    let non_git = TempDir::new("non-git");
    scope_failure(
        non_git.path(),
        ["push", "--main"],
        "run scope push from inside an existing Git repository",
    );

    let no_head = TempDir::new("no-head");
    run_git(no_head.path(), ["-c", "init.defaultBranch=main", "init"]);
    scope_failure(
        no_head.path(),
        ["push", "--main"],
        "create at least one Git commit before running scope push",
    );
}

#[test]
fn push_creates_missing_config_before_remote_lookup() {
    let dir = TempDir::new("missing-config");
    create_repo_with_head(dir.path());
    let stderr = scope_failure(
        dir.path(),
        ["push", "--main", "--no-review"],
        "no Scope Git remote found; pass --remote <name> or run scope init",
    );
    assert!(repo_config_path(dir.path()).unwrap().is_file());
    assert!(dir.path().join(".scope/RULES.md").is_file());
    assert!(!stderr.contains("Working tree has uncommitted changes."));
}

#[test]
fn push_validates_config_before_remote_lookup() {
    let dir = TempDir::new("invalid-config");
    create_repo_with_head(dir.path());
    let config_path = repo_config_path(dir.path()).unwrap();
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::write(
        config_path,
        r#"{
      "kind": "wrong", "version": 1,
      "visibility": { "default": "private", "rules": [] }
    }"#,
    )
    .unwrap();
    scope_failure(
        dir.path(),
        ["push", "--main", "--no-review"],
        "repo config kind must be scope.repo-config",
    );
}

#[test]
fn push_warns_about_dirty_state_before_remote_lookup() {
    let dir = configured_repo("dirty");
    fs::write(dir.path().join("README.md"), "uncommitted\n").unwrap();
    let stderr = scope_failure(
        dir.path(),
        ["push", "--main", "--no-review"],
        "no Scope Git remote found; pass --remote <name> or run scope init",
    );
    assert!(stderr.contains("Working tree has uncommitted changes."));
    assert!(stderr.contains("Only committed HEAD will be pushed to Scope."));
}

#[test]
fn push_requires_review_tty_before_remote_lookup() {
    let dir = configured_repo("review-non-tty");
    scope_failure(
        dir.path(),
        ["push", "--main"],
        "scope push review requires an interactive terminal",
    );
}

fn configured_repo(label: &str) -> TempDir {
    let dir = TempDir::new(label);
    create_repo_with_head(dir.path());
    let config_path = repo_config_path(dir.path()).unwrap();
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::write(
        config_path,
        r#"{
      "kind": "scope.repo-config", "version": 1,
      "visibility": { "default": "private", "rules": [] },
      "history": { "rewrites": [] }
    }"#,
    )
    .unwrap();
    dir
}

#[test]
fn push_requires_explicit_main_before_any_repository_changes() {
    let dir = TempDir::new("push-explicit-main");
    create_repo_with_head(dir.path());
    let output = scope_command(dir.path())
        .args(["--json", "push", "--no-review"])
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
    assert_eq!(value["code"], "bad_request");
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("scope request push")
    );
    assert!(!repo_config_path(dir.path()).unwrap().exists());
}

#[cfg(unix)]
#[test]
fn ctrl_c_during_delayed_login_validation_exits_before_publish() {
    let dir = configured_repo("push-cancel-http");
    let request_started = Arc::new(AtomicBool::new(false));
    let publish_requests = Arc::new(AtomicUsize::new(0));
    let server = TestServer::new(
        Router::new()
            .route(
                "/v1/session",
                get({
                    let request_started = Arc::clone(&request_started);
                    move || {
                        let request_started = Arc::clone(&request_started);
                        async move {
                            request_started.store(true, Ordering::Release);
                            tokio::time::sleep(Duration::from_secs(10)).await;
                            Json(session_response("usr_test", "owner", "owner@example.test"))
                        }
                    }
                }),
            )
            .route(
                "/v1/repos/owner/repo/push-intents",
                axum::routing::post({
                    let publish_requests = Arc::clone(&publish_requests);
                    move || {
                        publish_requests.fetch_add(1, Ordering::AcqRel);
                        async { Json(serde_json::json!({})) }
                    }
                }),
            ),
    );
    let remote = format!("{}/git/permissioned/owner/repo", server.api_url);
    run_git(dir.path(), ["remote", "add", "scope", &remote]);

    let mut master_fd = 0;
    let mut slave_fd = 0;
    let terminal_size = libc::winsize {
        ws_row: 24,
        ws_col: 80,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty initializes both file descriptors, which are immediately owned by File.
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &terminal_size,
            )
        },
        0
    );
    // SAFETY: openpty returned two new, valid, owned descriptors.
    let mut terminal_output = unsafe { File::from_raw_fd(master_fd) };
    let terminal = unsafe { File::from_raw_fd(slave_fd) };
    let mut command = server.command(dir.path());
    command
        .args(["push", "--main", "--no-review"])
        .stdin(terminal.try_clone().unwrap())
        .stdout(terminal.try_clone().unwrap())
        .stderr(terminal.try_clone().unwrap());
    let mut child = command.spawn().unwrap();
    drop(command);
    drop(terminal);
    let request_deadline = Instant::now() + Duration::from_secs(2);
    while !request_started.load(Ordering::Acquire) && Instant::now() < request_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(request_started.load(Ordering::Acquire));

    let cancelled_at = Instant::now();
    // SAFETY: child.id() names the live CLI subprocess created above.
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let exit_deadline = Instant::now() + Duration::from_secs(2);
    while child.try_wait().unwrap().is_none() && Instant::now() < exit_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if child.try_wait().unwrap().is_none() {
        let _ = child.kill();
        panic!("scope push did not cancel promptly during delayed HTTP");
    }
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(130), "{status:?}");
    assert!(cancelled_at.elapsed() < Duration::from_secs(2));
    assert_eq!(publish_requests.load(Ordering::Acquire), 0);
    let mut transcript = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match terminal_output.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => transcript.extend_from_slice(&buffer[..count]),
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) => panic!("read test terminal: {error}"),
        }
    }
    assert!(
        transcript
            .windows("Verifying login…".len())
            .any(|window| window == "Verifying login…".as_bytes()),
        "{}",
        String::from_utf8_lossy(&transcript)
    );
    assert!(transcript.ends_with(b"\r\x1b[2K"), "{transcript:?}");
}
