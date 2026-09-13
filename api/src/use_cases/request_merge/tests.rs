use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn explicit_request_base_preserves_private_main_files() {
    let repo = temp_repo_path("preserves-private");
    run_git(
        None,
        &[
            "init",
            "--initial-branch=main",
            repo.to_string_lossy().as_ref(),
        ],
        "initializing merge test repository",
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

    fs::write(repo.join("public.txt"), "public base\n").unwrap();
    commit_all(&repo, "public base");
    let request_base = oid(&repo, "HEAD");

    fs::write(repo.join("private.txt"), "private main\n").unwrap();
    commit_all(&repo, "private main change");
    let current_main = oid(&repo, "HEAD");

    run_git(
        Some(&repo),
        &["switch", "--create", "request", &request_base],
        "creating request branch",
    )
    .unwrap();
    fs::write(repo.join("public.txt"), "public request\n").unwrap();
    commit_all(&repo, "request change");
    let request_head = oid(&repo, "HEAD");

    let merged = merge_main_oid(
        &repo,
        &request_base,
        &current_main,
        &request_head,
        "public-request",
    )
    .unwrap();
    assert_eq!(
        git_text(&repo, &["show", &format!("{merged}:private.txt")]),
        "private main\n"
    );
    assert_eq!(
        git_text(&repo, &["show", &format!("{merged}:public.txt")]),
        "public request\n"
    );
    let parents = git_text(&repo, &["show", "-s", "--format=%P", &merged]);
    assert_eq!(
        parents.split_ascii_whitespace().collect::<Vec<_>>(),
        [current_main.as_str(), request_head.as_str()]
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn explicit_request_base_merges_non_overlapping_file_edits() {
    let repo = temp_repo_path("content-merge");
    run_git(
        None,
        &[
            "init",
            "--initial-branch=main",
            repo.to_string_lossy().as_ref(),
        ],
        "initializing merge test repository",
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

    fs::write(repo.join("shared.txt"), "top\nmiddle\nbottom\n").unwrap();
    commit_all(&repo, "shared base");
    let request_base = oid(&repo, "HEAD");

    fs::write(repo.join("shared.txt"), "main top\nmiddle\nbottom\n").unwrap();
    commit_all(&repo, "main edit");
    let current_main = oid(&repo, "HEAD");

    run_git(
        Some(&repo),
        &["switch", "--create", "request", &request_base],
        "creating request branch",
    )
    .unwrap();
    fs::write(repo.join("shared.txt"), "top\nmiddle\nrequest bottom\n").unwrap();
    commit_all(&repo, "request edit");
    let request_head = oid(&repo, "HEAD");

    let merged = merge_main_oid(
        &repo,
        &request_base,
        &current_main,
        &request_head,
        "content-merge",
    )
    .unwrap();
    assert_eq!(
        git_text(&repo, &["show", &format!("{merged}:shared.txt")]),
        "main top\nmiddle\nrequest bottom\n"
    );
    let _ = fs::remove_dir_all(repo);
}

fn commit_all(repo: &Path, message: &str) {
    run_git(Some(repo), &["add", "."], "staging merge test files").unwrap();
    run_git(
        Some(repo),
        &["commit", "-m", message],
        "committing merge test files",
    )
    .unwrap();
}

fn oid(repo: &Path, revision: &str) -> String {
    git_text(repo, &["rev-parse", revision]).trim().to_string()
}

fn git_text(repo: &Path, args: &[&str]) -> String {
    let output = run_git_output(Some(repo), args, "reading merge test repository").unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn temp_repo_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "scope-vcs-request-merge-{label}-{}-{nonce}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    path
}
