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
fn invalid_watch_timeout_is_rejected_before_network_access() {
    let dir = TempDir::new("run-timeout");
    scope_failure_with_code(
        dir.path(),
        ["run", "watch", "run-123", "--timeout", "0"],
        "invalid value",
        2,
    );
}
