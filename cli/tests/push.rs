mod support;

use scope_cli::repo_config::repo_config_path;
use std::fs;
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
