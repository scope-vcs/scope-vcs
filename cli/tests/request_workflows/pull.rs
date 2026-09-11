use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn pull_fast_forwards_a_checked_out_request_alias_and_preserves_divergence() {
    let dir = TempDir::new("request-pull-alias");
    create_repo_with_head(dir.path());
    let head = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    let bare = TempDir::new("request-pull-bare");
    run_git(bare.path(), ["init", "--bare"]);
    run_git(
        dir.path(),
        [
            "push",
            bare.path().to_str().unwrap(),
            "HEAD:refs/heads/fix-one",
        ],
    );
    let mut detail = request();
    detail["base_main_oid"] = head.clone().into();
    detail["head_oid"] = head.clone().into();
    detail["permissions"]["can_pull_branch"] = true.into();
    detail["mergeability"]["current_main_oid"] = head.clone().into();
    detail["mergeability"]["request_head_oid"] = head.clone().into();
    let server = FixtureServer::with_request(detail);
    let permissioned = format!("{}/git/permissioned/owner/repo", server.server.api_url);
    run_git(dir.path(), ["remote", "add", "scope", &permissioned]);

    // Translate only transport invocations. Repository discovery still sees the
    // real Scope URL, while Git fetch reads an actual bare repository.
    let shim = TempDir::new("pull-git-transport");
    let shim_path = shim.path().join("git");
    fs::write(&shim_path, r#"#!/bin/bash
for arg in "$@"; do
  if [[ "$arg" == "fetch" ]]; then
    exec "$SCOPE_TEST_REAL_GIT" -c "url.$SCOPE_TEST_FILE_URL.insteadOf=$SCOPE_TEST_PERMISSIONED_URL" "$@"
  fi
done
exec "$SCOPE_TEST_REAL_GIT" "$@"
"#).unwrap();
    fs::set_permissions(&shim_path, fs::Permissions::from_mode(0o700)).unwrap();
    let existing_path = std::env::var_os("PATH").unwrap();
    let real_git = std::env::split_paths(&existing_path)
        .map(|dir| dir.join("git"))
        .find(|path| path.is_file())
        .unwrap();
    let test_path = std::env::join_paths(
        std::iter::once(shim.path().to_path_buf()).chain(std::env::split_paths(&existing_path)),
    )
    .unwrap();
    let file_url = reqwest::Url::from_directory_path(bare.path())
        .unwrap()
        .to_string();
    let command = || {
        let mut command = server.command(dir.path());
        command
            .env("PATH", &test_path)
            .env("SCOPE_TEST_REAL_GIT", &real_git)
            .env("SCOPE_TEST_PERMISSIONED_URL", &permissioned)
            .env("SCOPE_TEST_FILE_URL", &file_url);
        command
    };
    success(
        command()
            .args([
                "--json",
                "request",
                "checkout",
                "--request",
                "req_one",
                "--branch",
                "local-alias",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(
        git_stdout(dir.path(), &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "scope/fix-one"
    );

    let writer = TempDir::new("request-pull-writer");
    run_git(
        writer.path(),
        [
            "clone",
            "--branch",
            "fix-one",
            bare.path().to_str().unwrap(),
            ".",
        ],
    );
    fs::write(writer.path().join("remote.txt"), "remote update").unwrap();
    run_git(writer.path(), ["add", "."]);
    commit_all(writer.path(), "advance remote request");
    run_git(writer.path(), ["push", "origin", "fix-one"]);
    let advanced = git_stdout(writer.path(), &["rev-parse", "HEAD"]);
    let pulled = success(command().args(["--json", "pull"]).output().unwrap());
    assert_eq!(pulled["result"]["branch_moved"], true);
    assert_eq!(git_stdout(dir.path(), &["rev-parse", "HEAD"]), advanced);
    assert_eq!(
        git_stdout(dir.path(), &["branch", "--show-current"]),
        "local-alias"
    );

    fs::write(dir.path().join("local.txt"), "local commit").unwrap();
    run_git(dir.path(), ["add", "."]);
    commit_all(dir.path(), "local divergence");
    let local_head = git_stdout(dir.path(), &["rev-parse", "HEAD"]);
    fs::write(writer.path().join("remote.txt"), "second remote update").unwrap();
    run_git(writer.path(), ["add", "."]);
    commit_all(writer.path(), "remote divergence");
    run_git(writer.path(), ["push", "origin", "fix-one"]);
    let output = command().args(["--json", "pull"]).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(git_stdout(dir.path(), &["rev-parse", "HEAD"]), local_head);
    assert_eq!(
        fs::read_to_string(dir.path().join("local.txt")).unwrap(),
        "local commit"
    );

    run_git(
        dir.path(),
        ["config", "--unset", "branch.local-alias.remote"],
    );
    let untracked = success(
        command()
            .args(["--json", "pull", "--remote", "scope"])
            .output()
            .unwrap(),
    );
    assert_eq!(untracked["result"]["branch_moved"], false);
    assert_eq!(git_stdout(dir.path(), &["rev-parse", "HEAD"]), local_head);
}
