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

#[test]
fn content_conflict_is_the_only_typed_merge_conflict() {
    let repo = temp_repo_path("typed-content-conflict");
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

    fs::write(repo.join("shared.txt"), "base\n").unwrap();
    commit_all(&repo, "shared base");
    let request_base = oid(&repo, "HEAD");

    fs::write(repo.join("shared.txt"), "main\n").unwrap();
    commit_all(&repo, "main edit");
    let current_main = oid(&repo, "HEAD");

    run_git(
        Some(&repo),
        &["switch", "--create", "request", &request_base],
        "creating request branch",
    )
    .unwrap();
    fs::write(repo.join("shared.txt"), "request\n").unwrap();
    commit_all(&repo, "request edit");
    let request_head = oid(&repo, "HEAD");

    let failure = merge_main_oid_for_execution(
        &repo,
        &request_base,
        &current_main,
        &request_head,
        "content-conflict",
    )
    .unwrap_err();
    assert!(matches!(failure, MergeMainFailure::Conflict(_)));

    let non_conflict_failure = merge_main_oid_for_execution(
        &repo,
        "ffffffffffffffffffffffffffffffffffffffff",
        &current_main,
        &request_head,
        "missing-base",
    )
    .unwrap_err();
    assert!(matches!(non_conflict_failure, MergeMainFailure::Other(_)));

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn merge_preserves_rename_executable_mode_and_binary_content() {
    let repo = temp_repo_path("rename-mode-binary");
    run_git(
        None,
        &["init", "-b", "main", repo.to_string_lossy().as_ref()],
        "init",
    )
    .unwrap();
    run_git(Some(&repo), &["config", "user.name", "Test"], "config name").unwrap();
    run_git(
        Some(&repo),
        &["config", "user.email", "test@scope.local"],
        "config email",
    )
    .unwrap();
    fs::write(repo.join("old.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
    fs::write(repo.join("script.sh"), "#!/bin/sh\n").unwrap();
    fs::write(repo.join("data.bin"), [0, 1, 2, 255]).unwrap();
    commit_all(&repo, "base");
    let base = oid(&repo, "HEAD");

    fs::rename(repo.join("old.txt"), repo.join("new.txt")).unwrap();
    commit_all(&repo, "rename on main");
    let main = oid(&repo, "HEAD");

    run_git(
        Some(&repo),
        &["switch", "-c", "request", &base],
        "request branch",
    )
    .unwrap();
    fs::write(repo.join("old.txt"), "one\ntwo\nrequest\nfour\nfive\n").unwrap();
    fs::write(repo.join("data.bin"), [0, 9, 2, 255]).unwrap();
    run_git(Some(&repo), &["add", "."], "stage request").unwrap();
    run_git(
        Some(&repo),
        &["update-index", "--chmod=+x", "script.sh"],
        "mode change",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &["commit", "-m", "request edit"],
        "commit request",
    )
    .unwrap();
    let request = oid(&repo, "HEAD");

    let merged = merge_main_oid(&repo, &base, &main, &request, "request").unwrap();
    assert_eq!(
        git_text(&repo, &["show", &format!("{merged}:new.txt")]),
        "one\ntwo\nrequest\nfour\nfive\n"
    );
    let blob = run_git_output(
        Some(&repo),
        &["show", &format!("{merged}:data.bin")],
        "binary",
    )
    .unwrap();
    assert_eq!(blob.stdout, [0, 9, 2, 255]);
    assert!(git_text(&repo, &["ls-tree", &merged, "script.sh"]).starts_with("100755 blob "));
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn recorded_base_controls_merge_even_when_git_finds_a_newer_common_ancestor() {
    let repo = temp_repo_path("recorded-base");
    run_git(
        None,
        &["init", "-b", "main", repo.to_string_lossy().as_ref()],
        "init",
    )
    .unwrap();
    run_git(Some(&repo), &["config", "user.name", "Test"], "config name").unwrap();
    run_git(
        Some(&repo),
        &["config", "user.email", "test@scope.local"],
        "config email",
    )
    .unwrap();
    fs::write(repo.join("shared.txt"), "base\n").unwrap();
    commit_all(&repo, "recorded base");
    let recorded_base = oid(&repo, "HEAD");
    fs::write(repo.join("shared.txt"), "intermediate\n").unwrap();
    commit_all(&repo, "newer common ancestor");
    let newer_base = oid(&repo, "HEAD");
    run_git(Some(&repo), &["switch", "-c", "request"], "request branch").unwrap();
    fs::write(repo.join("request.txt"), "request\n").unwrap();
    commit_all(&repo, "request");
    let request = oid(&repo, "HEAD");
    run_git(Some(&repo), &["switch", "main"], "main branch").unwrap();
    fs::write(repo.join("shared.txt"), "latest\n").unwrap();
    commit_all(&repo, "main changed again");
    let main = oid(&repo, "HEAD");

    assert_eq!(
        git_text(&repo, &["merge-base", &main, &request]).trim(),
        newer_base
    );
    assert!(matches!(
        merge_main_oid_for_execution(&repo, &recorded_base, &main, &request, "request"),
        Err(MergeMainFailure::Conflict(_))
    ));
    assert!(merge_main_oid(&repo, &newer_base, &main, &request, "request").is_ok());
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
