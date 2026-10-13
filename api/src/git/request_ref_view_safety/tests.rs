use super::*;
use crate::git::test_repo::TestRepo;
use std::fs;

#[test]
fn view_request_range_is_oldest_first_with_exact_git_facts() {
    let repo = initialized_repo();
    fs::write(repo.join("agent.txt"), "base\n").unwrap();
    repo.commit_all("view base");
    let view_base = repo.oid("HEAD");
    run_git(
        Some(&repo),
        &["update-ref", VIEW_REQUEST_BASE_REF, &view_base],
        "recording view request base",
    )
    .unwrap();

    fs::write(repo.join("agent.txt"), "first\n").unwrap();
    repo.commit_all("first request commit");
    let first = repo.oid("HEAD");
    fs::write(repo.join("second.txt"), "second\n").unwrap();
    repo.commit_all("second request commit");
    let second = repo.oid("HEAD");

    let commits = commits_after(&repo, VIEW_REQUEST_BASE_REF, &second).unwrap();
    assert_eq!(commits, [first.clone(), second.clone()]);

    let first_path = ScopePath::parse("/agent.txt").unwrap();
    let first_fact = native_request_commit(&repo, &first, vec![first_path.clone()]).unwrap();
    assert_eq!(first_fact.oid, first);
    assert_eq!(first_fact.parent_oids, [view_base]);
    assert_eq!(first_fact.changed_paths, [first_path]);
    assert_eq!(
        first_fact.tree_oid,
        repo.oid(&format!("{}^{{tree}}", first_fact.oid))
    );

    let second_path = ScopePath::parse("/second.txt").unwrap();
    let second_fact = native_request_commit(&repo, &second, vec![second_path.clone()]).unwrap();
    assert_eq!(second_fact.oid, second);
    assert_eq!(second_fact.parent_oids, [first_fact.oid]);
    assert_eq!(second_fact.changed_paths, [second_path]);
    assert_eq!(
        second_fact.tree_oid,
        repo.oid(&format!("{}^{{tree}}", second_fact.oid))
    );
}

#[test]
fn view_request_range_rejects_parent_outside_view_history_or_range() {
    let repo = initialized_repo();
    fs::write(repo.join("agent.txt"), "base\n").unwrap();
    repo.commit_all("view base");
    let view_base = repo.oid("HEAD");
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
    repo.commit_all("external commit");

    run_git(
        Some(&repo),
        &["switch", "--create", "request", &view_base],
        "creating request branch",
    )
    .unwrap();
    fs::write(repo.join("request.txt"), "request\n").unwrap();
    repo.commit_all("request commit");
    let request_commit = repo.oid("HEAD");
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
    let request_head = repo.oid("HEAD");

    let error =
        validated_view_parent_oids("Agent", &repo, &[request_commit, request_head]).unwrap_err();
    assert!(
        error
            .public_message()
            .contains("Agent request contains a parent outside Agent history")
    );
}

#[test]
fn merge_validation_rejects_request_without_current_view_head() {
    let repo = initialized_repo();
    fs::write(repo.join("agent.txt"), "base\n").unwrap();
    repo.commit_all("view base");
    let original_base = repo.oid("HEAD");

    run_git(
        Some(&repo),
        &["switch", "--create", "request", &original_base],
        "creating request branch",
    )
    .unwrap();
    fs::write(repo.join("request.txt"), "request\n").unwrap();
    repo.commit_all("request change");
    let request_head = repo.oid("HEAD");

    run_git(Some(&repo), &["switch", "main"], "returning to view main").unwrap();
    fs::write(repo.join("main.txt"), "advanced\n").unwrap();
    repo.commit_all("advance view main");
    let current_view_head = repo.oid("HEAD");
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
}

#[test]
fn merge_path_validation_ignores_rules_inherited_from_current_view_main() {
    let repo = initialized_repo();
    fs::write(repo.join("agent.txt"), "base\n").unwrap();
    repo.commit_all("view base");
    let original_base = repo.oid("HEAD");

    run_git(
        Some(&repo),
        &["switch", "--create", "request", &original_base],
        "creating request branch",
    )
    .unwrap();
    fs::write(repo.join("request.txt"), "request\n").unwrap();
    repo.commit_all("request change");

    run_git(Some(&repo), &["switch", "main"], "returning to view main").unwrap();
    fs::write(repo.join(".scope/RULES.md"), "maintainer rules\n").unwrap();
    repo.commit_all("update maintainer rules");
    let current_view_head = repo.oid("HEAD");
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
    let merge_oid = repo.oid("HEAD");

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
    let amended_merge_oid = repo.oid("HEAD");

    assert_eq!(
        request_changed_paths(&repo, &amended_merge_oid).unwrap(),
        [".scope/RULES.md", "request.txt"]
    );
}

fn initialized_repo() -> TestRepo {
    let repo = TestRepo::init();
    fs::create_dir_all(repo.join(".scope")).unwrap();
    fs::write(repo.join(".scope/RULES.md"), []).unwrap();
    repo
}
