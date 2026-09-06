mod support;
use serde_json::Value;
use std::fs;
use support::*;

#[test]
fn offline_status_retains_local_facts_without_creating_scope_state() {
    let dir = TempDir::new("status-offline");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("README.md"), "dirty\n").unwrap();
    let output = scope_command(dir.path())
        .args(["--json", "status", "--offline"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["command"], "status");
    assert_eq!(value["result"]["local"]["dirty"], true);
    assert_eq!(value["result"]["offline"], true);
    assert!(value["result"]["local"]["head_oid"].as_str().is_some());
    assert!(!dir.path().join(".git/scope").exists());
}

#[test]
fn doctor_reports_incomplete_setup_without_repairing_it() {
    let dir = TempDir::new("doctor-read-only");
    create_repo_with_head(dir.path());
    let output = scope_command(dir.path())
        .args(["--json", "doctor", "--offline"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["result"]["healthy"], false);
    assert!(
        value["result"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "visibility" && d["state"] == "problem")
    );
    assert!(!dir.path().join(".git/scope").exists());
}

#[test]
fn repository_api_setting_is_used_without_an_environment_override() {
    let dir = TempDir::new("context-api-setting");
    create_repo_with_head(dir.path());
    run_git(
        dir.path(),
        ["config", "scope.apiUrl", "https://staging.scope.example"],
    );
    let output = scope_command(dir.path())
        .env_remove("SCOPE_API_URL")
        .env_remove("SCOPE_API_PUBLIC_URL")
        .args(["--json", "status", "--offline"])
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["result"]["api_url"], "https://staging.scope.example");
}

#[test]
fn noninteractive_login_and_identity_fail_with_authentication_category() {
    let dir = TempDir::new("noninteractive-auth");
    for args in [
        ["--json", "--non-interactive", "login"],
        ["--json", "--non-interactive", "whoami"],
    ] {
        let output = scope_command(dir.path()).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(3), "{output:?}");
        assert!(output.stdout.is_empty());
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["code"], "unauthorized");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("Opening browser"));
    }
}

#[test]
fn shell_completions_describe_new_commands_and_global_controls() {
    let dir = TempDir::new("completions");
    let output = scope_command(dir.path())
        .args(["completions", "bash"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let body = String::from_utf8(output.stdout).unwrap();
    assert!(body.contains("visibility"));
    assert!(body.contains("--non-interactive"));
    assert!(body.contains("--main"));
}
