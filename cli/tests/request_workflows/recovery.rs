use super::*;

#[test]
fn request_start_metadata_failure_can_retry_push_without_creating_another_request() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("request-start-recovery");
    create_repo_with_head(dir.path());
    let head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let bare = TempDir::new("request-recovery-bare");
    run_git(bare.path(), ["init", "--bare"]);
    run_git(dir.path(), ["push", bare.path().to_str().unwrap(), "main"]);
    let mut detail = request();
    detail["base_main_oid"] = head.clone().into();
    detail["head_oid"] = head.clone().into();
    detail["mergeability"]["current_main_oid"] = head.clone().into();
    detail["mergeability"]["request_head_oid"] = head.clone().into();
    detail["permissions"]["can_pull_branch"] = true.into();
    let server = FixtureServer::with_request(detail);
    let public = format!("{}/git/public/owner/repo", server.server.api_url);
    let permissioned = format!("{}/git/permissioned/owner/repo", server.server.api_url);
    let file_url = reqwest::Url::from_directory_path(bare.path())
        .unwrap()
        .to_string();

    run_git(dir.path(), ["remote", "add", "scope", &public]);
    run_git(
        dir.path(),
        ["remote", "set-url", "--push", "scope", &permissioned],
    );

    let shim = TempDir::new("request-git-transport");
    let shim_path = shim.path().join("git");
    fs::write(
        &shim_path,
        r#"#!/bin/bash
args=()
for arg in "$@"; do
  if [[ "$arg" == "$SCOPE_TEST_PUBLIC_URL" || "$arg" == "$SCOPE_TEST_PERMISSIONED_URL" ]]; then
    args+=("$SCOPE_TEST_FILE_URL")
  else
    args+=("$arg")
  fi
done
exec "$SCOPE_TEST_REAL_GIT" "${args[@]}"
"#,
    )
    .unwrap();
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
    let command = || {
        let mut command = server.command(dir.path());
        command
            .env("PATH", &test_path)
            .env("SCOPE_TEST_REAL_GIT", &real_git)
            .env("SCOPE_TEST_PUBLIC_URL", &public)
            .env("SCOPE_TEST_PERMISSIONED_URL", &permissioned)
            .env("SCOPE_TEST_FILE_URL", &file_url);
        command
    };
    let hook = dir.path().join(".git/hooks/post-checkout");
    fs::write(&hook, "#!/bin/sh\n: > .git/config.lock\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    let output = command()
        .args(["--json", "request", "start", "fix-one"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    let error: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    assert_eq!(error["recovery"]["request_id"], "req_one");
    assert_eq!(error["recovery"]["failed_step"], "save_local_metadata");
    assert_eq!(error["recovery"]["remote_push_confirmed"], false);
    assert_eq!(
        error["recovery"]["follow_up_command"],
        json!([
            "scope",
            "request",
            "checkout",
            "--remote",
            "scope",
            "--request",
            "req_one",
            "--branch",
            "fix-one"
        ])
    );
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("scope request checkout --remote scope --request req_one --branch fix-one")
    );
    assert_eq!(
        git_stdout(dir.path(), ["branch", "--show-current"]),
        "fix-one"
    );
    assert_eq!(git_stdout(dir.path(), ["rev-parse", "HEAD"]), head);
    fs::remove_file(hook).unwrap();
    fs::remove_file(dir.path().join(".git/config.lock")).unwrap();
    let output = command()
        .args([
            "--json",
            "request",
            "push",
            "--remote",
            "scope",
            "--request",
            "req_one",
        ])
        .output()
        .unwrap();
    assert_eq!(success(output)["command"], "request.push");
    assert_eq!(
        git_stdout(bare.path(), ["rev-parse", "refs/heads/fix-one"]),
        head
    );
    assert!(
        !Command::new("git")
            .current_dir(dir.path())
            .args(["config", "--get", "branch.fix-one.scopeRequestId"])
            .status()
            .unwrap()
            .success()
    );
    let output = command()
        .args([
            "--json",
            "request",
            "checkout",
            "--remote",
            "scope",
            "--request",
            "req_one",
            "--branch",
            "fix-one",
        ])
        .output()
        .unwrap();
    assert_eq!(success(output)["command"], "request.checkout");
    assert_eq!(
        git_stdout(
            dir.path(),
            ["config", "--get", "branch.fix-one.scopeRequestId"]
        ),
        "req_one"
    );
    run_git(
        dir.path(),
        ["config", "branch.fix-one.scopeRequestId", "req_old"],
    );
    run_git(
        dir.path(),
        ["config", "branch.fix-one.scopeRequestOwner", "old-owner"],
    );
    run_git(
        dir.path(),
        ["config", "branch.fix-one.scopeRequestRepo", "old-repo"],
    );
    run_git(
        dir.path(),
        ["config", "branch.fix-one.scopeRequestRemote", "origin"],
    );
    run_git(
        dir.path(),
        ["config", "branch.fix-one.scopeRequestAudience", "private"],
    );
    run_git(dir.path(), ["config", "branch.fix-one.remote", "origin"]);
    run_git(
        dir.path(),
        ["config", "branch.fix-one.merge", "refs/heads/old-request"],
    );
    run_git(
        dir.path(),
        ["update-ref", "-d", "refs/remotes/scope/fix-one"],
    );
    let old_metadata = git_stdout(
        dir.path(),
        ["config", "--get-regexp", "^branch\\.fix-one\\.scopeRequest"],
    );
    let output = command()
        .args([
            "--json",
            "request",
            "push",
            "--remote",
            "scope",
            "--request",
            "req_one",
        ])
        .output()
        .unwrap();
    assert_eq!(success(output)["command"], "request.push");
    assert_eq!(
        git_stdout(
            dir.path(),
            ["config", "--get-regexp", "^branch\\.fix-one\\.scopeRequest"]
        ),
        old_metadata
    );
    assert_eq!(
        git_stdout(
            dir.path(),
            ["config", "--get", "branch.fix-one.scopeRequestId"]
        ),
        "req_old"
    );
    assert_eq!(
        git_stdout(dir.path(), ["config", "--get", "branch.fix-one.remote"]),
        "origin"
    );
    assert_eq!(
        git_stdout(dir.path(), ["config", "--get", "branch.fix-one.merge"]),
        "refs/heads/old-request"
    );
    assert_eq!(
        git_stdout(dir.path(), ["rev-parse", "refs/remotes/scope/fix-one"]),
        head
    );
    run_git(
        dir.path(),
        ["config", "branch.fix-one.scopeRequestId", "req_one"],
    );
    run_git(
        dir.path(),
        ["config", "branch.fix-one.scopeRequestOwner", "owner"],
    );
    run_git(
        dir.path(),
        ["config", "branch.fix-one.scopeRequestRepo", "repo"],
    );
    run_git(
        dir.path(),
        ["config", "branch.fix-one.scopeRequestRemote", "scope"],
    );
    run_git(
        dir.path(),
        ["update-ref", "-d", "refs/remotes/scope/fix-one"],
    );
    let output = command()
        .args(["--json", "request", "push"])
        .output()
        .unwrap();
    assert_eq!(success(output)["command"], "request.push");
    assert_eq!(
        git_stdout(
            dir.path(),
            ["config", "--get", "branch.fix-one.scopeRequestId"]
        ),
        "req_one"
    );
    assert_eq!(
        git_stdout(dir.path(), ["config", "--get", "branch.fix-one.remote"]),
        "origin"
    );
    assert_eq!(
        git_stdout(dir.path(), ["rev-parse", "refs/remotes/scope/fix-one"]),
        head
    );
    assert_eq!(
        server
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.as_str() == "POST request")
            .count(),
        1
    );
}
