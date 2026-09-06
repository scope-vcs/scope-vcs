mod support;
use support::*;

#[test]
fn run_control_commands_require_an_id_before_repository_or_auth_work() {
    let dir = TempDir::new("run-control-id");
    for command in ["show", "watch", "logs", "cancel", "retry"] {
        scope_failure_with_code(dir.path(), ["run", command], "<RUN_ID>", 2);
    }
}

#[test]
fn run_help_exposes_real_subcommands_and_scoped_options() {
    let dir = TempDir::new("run-help");
    let output = scope_command(dir.path())
        .args(["run", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    for command in [
        "start",
        "workflows",
        "list",
        "show",
        "watch",
        "logs",
        "cancel",
        "retry",
    ] {
        assert!(stdout.contains(command), "{stdout}");
    }
    let output = scope_command(dir.path())
        .args(["run", "watch", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("--timeout"), "{stdout}");
    assert!(stdout.contains("--after"), "{stdout}");
}

#[test]
fn invalid_watch_timeout_is_rejected_before_network_access() {
    let dir = TempDir::new("run-timeout");
    scope_failure_with_code(
        dir.path(),
        ["run", "watch", "run-123", "--timeout", "0"],
        "invalid value",
        2,
    );
}
