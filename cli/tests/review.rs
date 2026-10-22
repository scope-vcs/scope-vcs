mod support;

use support::*;

#[test]
fn review_can_start_before_first_commit() {
    let dir = TempDir::new("no-head");
    run_git(dir.path(), ["-c", "init.defaultBranch=main", "init"]);

    let output = scope_command(dir.path())
        .args(["visibility", "edit"])
        .output()
        .unwrap();

    assert_failure(&output, "scope visibility edit without HEAD");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("scope visibility edit requires an interactive terminal"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("create at least one Git commit before running scope visibility edit"),
        "{stderr}"
    );
}
