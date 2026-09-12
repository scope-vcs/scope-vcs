mod support;

use support::*;

#[test]
fn request_attachments_are_valid_content_before_login() {
    let dir = TempDir::new("request-attachment-content");
    create_repo_with_head(dir.path());

    for args in [
        vec![
            "request", "edit", "--attach", "shot.png", "--attach", "clip.mov",
        ],
        vec!["request", "discussion", "start", "--attach", "shot.png"],
        vec![
            "request",
            "discussion",
            "reply",
            "dsc_one",
            "--attach",
            "clip.mov",
        ],
        vec![
            "request",
            "discussion",
            "reopen",
            "dsc_one",
            "--attach",
            "clip.mov",
        ],
    ] {
        let output = scope_command(dir.path()).args(&args).output().unwrap();
        assert_failure(&output, "attachment-only request command");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!stderr.contains("required arguments"), "{args:?}: {stderr}");
        assert!(stderr.contains("scope login"), "{args:?}: {stderr}");
    }

    scope_failure(
        dir.path(),
        ["request", "edit", "--title", "Updated", "--wait"],
        "--attach <PATH>",
    );
}

#[test]
fn json_usage_errors_use_the_shared_schema_and_exit_two() {
    let dir = TempDir::new("request-json-usage");
    create_repo_with_head(dir.path());

    let output = scope_command(dir.path())
        .args([
            "--json", "request", "rate", "--score", "6", "--reason", "Invalid",
        ])
        .output()
        .unwrap();

    assert_failure(&output, "request JSON usage error");
    assert!(output.stdout.is_empty());
    assert_eq!(output.status.code(), Some(2));
    let error: scope_api_contract::ErrorResponse = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error.code, scope_api_contract::ErrorCode::BadRequest);
    assert!(!error.retryable);
}

#[test]
fn global_json_supports_local_rule_sync_results() {
    let dir = TempDir::new("json-command-scope");
    create_repo_with_head(dir.path());
    let output = scope_command(dir.path())
        .args(["--json", "rules", "sync"])
        .output()
        .unwrap();
    assert_success(&output, "rules sync JSON");
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["command"], "rules.sync");
    assert!(value["result"]["changed_paths"].is_array());
    assert!(output.stderr.is_empty());
}

#[test]
fn json_mode_preserves_successful_help_and_version_control_flow() {
    let dir = TempDir::new("json-help");
    create_repo_with_head(dir.path());

    for args in [
        vec!["--json", "--help"],
        vec!["request", "--json", "--help"],
        vec!["--json", "--version"],
    ] {
        let output = scope_command(dir.path()).args(&args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty(), "{args:?}");
        assert!(output.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn request_submit_reaches_auth_without_extra_arguments() {
    let dir = TempDir::new("submit-request");
    create_repo_with_head(dir.path());

    let output = scope_command(dir.path())
        .args(["request", "submit"])
        .output()
        .unwrap();
    assert_failure(&output, "scope request submit");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("required arguments"), "{stderr}");
    assert!(stderr.contains("scope login"), "{stderr}");
    assert!(!stderr.contains("start browser login"), "{stderr}");
}

#[test]
fn every_request_command_accepts_the_global_json_mode_and_returns_json_failures() {
    let dir = TempDir::new("request-json");
    create_repo_with_head(dir.path());

    let commands = [
        vec!["--json", "request", "start", "change"],
        vec!["request", "push", "--json"],
        vec!["--json", "request", "submit", "--yes"],
        vec!["request", "edit", "--title", "Updated", "--json"],
        vec!["--json", "request", "invite", "river"],
        vec!["request", "uninvite", "river", "--json"],
        vec!["--json", "request", "leave"],
        vec!["request", "merge", "--yes", "--json"],
        vec![
            "--json", "request", "rate", "--score", "5", "--reason", "Clear",
        ],
        vec![
            "request",
            "discussion",
            "start",
            "--body",
            "Question",
            "--json",
        ],
        vec![
            "request",
            "discussion",
            "reply",
            "dsc_one",
            "--body",
            "Answer",
            "--json",
        ],
        vec!["request", "discussion", "resolve", "dsc_one", "--json"],
        vec![
            "request",
            "discussion",
            "reopen",
            "dsc_one",
            "--body",
            "New evidence",
            "--json",
        ],
        vec!["--json", "request", "checkout"],
        vec!["--json", "request", "diff"],
        vec!["--json", "request", "checks"],
        vec!["--json", "request", "show"],
        vec!["request", "list", "--json"],
        vec!["--json", "request", "status"],
        vec!["request", "close", "--yes", "--json"],
    ];
    for args in commands {
        let output = scope_command(dir.path()).args(&args).output().unwrap();
        assert_failure(&output, "request JSON output");
        assert!(
            output.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let stderr = String::from_utf8(output.stderr).unwrap();
        let error: scope_api_contract::ErrorResponse = serde_json::from_str(stderr.trim()).unwrap();
        assert_eq!(
            error.code,
            scope_api_contract::ErrorCode::Unauthorized,
            "{args:?}: {stderr}"
        );
        assert!(!error.retryable);
        assert_eq!(output.status.code(), Some(3));
    }
}
