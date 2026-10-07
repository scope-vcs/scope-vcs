use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn view_request_range_is_oldest_first_with_exact_git_facts() {
    let repo = initialized_repo("exact-range");
    fs::write(repo.join("agent.txt"), "base\n").unwrap();
    commit_all(&repo, "view base");
    let view_base = oid(&repo, "HEAD");
    run_git(
        Some(&repo),
        &["update-ref", VIEW_REQUEST_BASE_REF, &view_base],
        "recording view request base",
    )
    .unwrap();

    fs::write(repo.join("agent.txt"), "first\n").unwrap();
    commit_all(&repo, "first request commit");
    let first = oid(&repo, "HEAD");
    fs::write(repo.join("second.txt"), "second\n").unwrap();
    commit_all(&repo, "second request commit");
    let second = oid(&repo, "HEAD");

    let commits = commits_after(&repo, VIEW_REQUEST_BASE_REF, &second).unwrap();
    assert_eq!(commits, [first.clone(), second.clone()]);

    let first_path = ScopePath::parse("/agent.txt").unwrap();
    let first_fact = native_request_commit(&repo, &first, vec![first_path.clone()]).unwrap();
    assert_eq!(first_fact.oid, first);
    assert_eq!(first_fact.parent_oids, [view_base]);
    assert_eq!(first_fact.changed_paths, [first_path]);
    assert_eq!(
        first_fact.tree_oid,
        oid(&repo, &format!("{}^{{tree}}", first_fact.oid))
    );

    let second_path = ScopePath::parse("/second.txt").unwrap();
    let second_fact = native_request_commit(&repo, &second, vec![second_path.clone()]).unwrap();
    assert_eq!(second_fact.oid, second);
    assert_eq!(second_fact.parent_oids, [first_fact.oid]);
    assert_eq!(second_fact.changed_paths, [second_path]);
    assert_eq!(
        second_fact.tree_oid,
        oid(&repo, &format!("{}^{{tree}}", second_fact.oid))
    );

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn view_request_range_rejects_parent_outside_view_history_or_range() {
    let repo = initialized_repo("external-parent");
    fs::write(repo.join("agent.txt"), "base\n").unwrap();
    commit_all(&repo, "view base");
    let view_base = oid(&repo, "HEAD");
    run_git(
        Some(&repo),
        &["update-ref", VIEW_REQUEST_BASE_REF, &view_base],
        "recording view request base",
    )
    .unwrap();

    run_git(
        Some(&repo),
        &["switch", "--create", "external"],
        "creating external branch",
    )
    .unwrap();
    fs::write(repo.join("external.txt"), "external\n").unwrap();
    commit_all(&repo, "external commit");

    run_git(
        Some(&repo),
        &["switch", "--create", "request", &view_base],
        "creating request branch",
    )
    .unwrap();
    fs::write(repo.join("request.txt"), "request\n").unwrap();
    commit_all(&repo, "request commit");
    let request_commit = oid(&repo, "HEAD");
    run_git(
        Some(&repo),
        &[
            "merge",
            "--no-ff",
            "external",
            "-m",
            "merge external parent",
        ],
        "creating request merge with external parent",
    )
    .unwrap();
    let request_head = oid(&repo, "HEAD");

    let error =
        validated_view_parent_oids("Agent", &repo, &[request_commit, request_head]).unwrap_err();
    assert!(
        error
            .public_message()
            .contains("Agent request contains a parent outside Agent history")
    );

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn merge_validation_rejects_request_without_current_view_head() {
    let repo = initialized_repo("stale-view-head");
    fs::write(repo.join("agent.txt"), "base\n").unwrap();
    commit_all(&repo, "view base");
    let original_base = oid(&repo, "HEAD");

    run_git(
        Some(&repo),
        &["switch", "--create", "request", &original_base],
        "creating request branch",
    )
    .unwrap();
    fs::write(repo.join("request.txt"), "request\n").unwrap();
    commit_all(&repo, "request change");
    let request_head = oid(&repo, "HEAD");

    run_git(Some(&repo), &["switch", "main"], "returning to view main").unwrap();
    fs::write(repo.join("main.txt"), "advanced\n").unwrap();
    commit_all(&repo, "advance view main");
    let current_view_head = oid(&repo, "HEAD");
    run_git(
        Some(&repo),
        &["update-ref", VIEW_REQUEST_BASE_REF, &current_view_head],
        "recording advanced view request base",
    )
    .unwrap();

    assert_eq!(
        ensure_view_head_is_request_ancestor(&repo, &request_head)
            .unwrap_err()
            .public_message(),
        VIEW_MAIN_MOVED_ERROR
    );

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn merge_path_validation_ignores_rules_inherited_from_current_view_main() {
    let repo = initialized_repo("inherited-view-rules");
    fs::write(repo.join("agent.txt"), "base\n").unwrap();
    commit_all(&repo, "view base");
    let original_base = oid(&repo, "HEAD");

    run_git(
        Some(&repo),
        &["switch", "--create", "request", &original_base],
        "creating request branch",
    )
    .unwrap();
    fs::write(repo.join("request.txt"), "request\n").unwrap();
    commit_all(&repo, "request change");

    run_git(Some(&repo), &["switch", "main"], "returning to view main").unwrap();
    fs::write(repo.join(".scope/RULES.md"), "maintainer rules\n").unwrap();
    commit_all(&repo, "update maintainer rules");
    let current_view_head = oid(&repo, "HEAD");
    run_git(
        Some(&repo),
        &["update-ref", VIEW_REQUEST_BASE_REF, &current_view_head],
        "recording advanced view request base",
    )
    .unwrap();

    run_git(
        Some(&repo),
        &["switch", "request"],
        "returning to request branch",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &["merge", "--no-ff", "main", "-m", "merge current view main"],
        "merging current view main",
    )
    .unwrap();
    let merge_oid = oid(&repo, "HEAD");

    let paths = request_changed_paths(&repo, &merge_oid).unwrap();

    assert_eq!(paths, ["request.txt"]);

    fs::write(repo.join(".scope/RULES.md"), "request override\n").unwrap();
    run_git(
        Some(&repo),
        &["add", ".scope/RULES.md"],
        "staging request rules override",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &["commit", "--amend", "--no-edit"],
        "amending merge with request rules override",
    )
    .unwrap();
    let amended_merge_oid = oid(&repo, "HEAD");

    assert_eq!(
        request_changed_paths(&repo, &amended_merge_oid).unwrap(),
        [".scope/RULES.md", "request.txt"]
    );

    let _ = fs::remove_dir_all(repo);
}

fn initialized_repo(label: &str) -> PathBuf {
    let repo = temp_repo_path(label);
    run_git(
        None,
        &[
            "init",
            "--initial-branch=main",
            repo.to_string_lossy().as_ref(),
        ],
        "initializing view request safety test repository",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &["config", "user.name", "Test"],
        "configuring test name",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &["config", "user.email", "test@scope.local"],
        "configuring test email",
    )
    .unwrap();
    fs::create_dir_all(repo.join(".scope")).unwrap();
    fs::write(repo.join(".scope/RULES.md"), []).unwrap();
    repo
}

fn commit_all(repo: &Path, message: &str) {
    run_git(Some(repo), &["add", "."], "staging safety test files").unwrap();
    run_git(
        Some(repo),
        &["commit", "-m", message],
        "committing safety test files",
    )
    .unwrap();
}

fn oid(repo: &Path, revision: &str) -> String {
    git_stdout_text(repo, &["rev-parse", revision], "reading safety test oid")
        .unwrap()
        .trim()
        .to_string()
}

fn temp_repo_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "scope-vcs-view-request-safety-{label}-{}-{nonce}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    path
}
