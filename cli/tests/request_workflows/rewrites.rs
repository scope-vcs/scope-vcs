use super::*;

#[test]
fn adopted_branch_compares_with_scope_request_despite_external_upstream() {
    let dir = TempDir::new("adopted-request-comparison");
    create_repo_with_head(dir.path());
    let base = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let bare = TempDir::new("adopted-request-comparison-bare");
    run_git(bare.path(), ["init", "--bare"]);
    run_git(
        dir.path(),
        [
            "push",
            bare.path().to_str().unwrap(),
            "main",
            "main:refs/heads/fix-one",
        ],
    );
    run_git(dir.path(), ["switch", "--quiet", "-c", "work"]);
    run_git(
        dir.path(),
        [
            "remote",
            "add",
            "origin",
            "https://github.com/owner/repo.git",
        ],
    );
    run_git(
        dir.path(),
        ["update-ref", "refs/remotes/origin/main", &base],
    );
    run_git(
        dir.path(),
        ["branch", "--set-upstream-to=origin/main", "work"],
    );
    fs::write(dir.path().join("fix.txt"), "first\n").unwrap();
    run_git(dir.path(), ["add", "fix.txt"]);
    commit_all(dir.path(), "First request commit");

    let head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let mut started = request();
    started["base_main_oid"] = base.clone().into();
    started["head_oid"] = base.clone().into();
    started["mergeability"]["current_main_oid"] = base.clone().into();
    started["mergeability"]["request_head_oid"] = base.into();
    let mut pushed = started.clone();
    pushed["head_oid"] = head.clone().into();
    pushed["mergeability"]["request_head_oid"] = head.clone().into();
    let server = FixtureServer::with_request_states(started, pushed);
    let transport = BareRepoTransport::new(&server, dir.path(), bare.path());
    let start = success(
        transport
            .command(&server, dir.path())
            .args(["--json", "request", "start", "fix-one", "--current-branch"])
            .output()
            .unwrap(),
    );
    assert_eq!(start["result"]["request"]["head_oid"], head);
    assert_eq!(
        git_stdout(dir.path(), ["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/main"
    );

    let status = || {
        success(
            transport
                .command(&server, dir.path())
                .args(["--json", "status", "--offline"])
                .output()
                .unwrap(),
        )
    };
    let pushed = status();
    assert_eq!(
        pushed["result"]["local"]["comparison_ref"],
        "refs/remotes/scope/fix-one"
    );
    assert_eq!(pushed["result"]["local"]["unpushed_commits"], 0);
    assert_eq!(
        git_stdout(
            dir.path(),
            ["rev-list", "--count", "refs/remotes/scope/main..HEAD"]
        ),
        "1"
    );

    run_git(
        dir.path(),
        ["config", "--unset", "branch.work.scopeRequestName"],
    );
    let legacy_offline = status();
    assert!(legacy_offline["result"]["local"]["comparison_ref"].is_null());
    assert!(legacy_offline["result"]["local"]["unpushed_commits"].is_null());
    assert!(
        legacy_offline["result"]["diagnostics"]
            .to_string()
            .contains("scope request checkout --request req_one")
    );
    assert!(
        !legacy_offline["result"]["next_actions"]
            .to_string()
            .contains("Start a contribution")
    );
    let legacy_online = success(
        transport
            .command(&server, dir.path())
            .args(["--json", "status"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        legacy_online["result"]["local"]["comparison_ref"],
        "refs/remotes/scope/fix-one"
    );
    assert_eq!(legacy_online["result"]["local"]["unpushed_commits"], 0);
    assert!(
        !Command::new("git")
            .current_dir(dir.path())
            .args(["config", "--get", "branch.work.scopeRequestName"])
            .status()
            .unwrap()
            .success()
    );
    run_git(
        dir.path(),
        ["config", "branch.work.scopeRequestName", "fix-one"],
    );

    fs::write(dir.path().join("fix.txt"), "second\n").unwrap();
    run_git(dir.path(), ["add", "fix.txt"]);
    commit_all(dir.path(), "Second request commit");
    let local = status();
    assert_eq!(
        local["result"]["local"]["comparison_ref"],
        "refs/remotes/scope/fix-one"
    );
    assert_eq!(local["result"]["local"]["unpushed_commits"], 1);

    run_git(
        dir.path(),
        ["update-ref", "-d", "refs/remotes/scope/fix-one"],
    );
    let missing_ref = status();
    assert_eq!(
        missing_ref["result"]["local"]["comparison_ref"],
        "refs/remotes/scope/fix-one"
    );
    assert!(missing_ref["result"]["local"]["unpushed_commits"].is_null());

    run_git(dir.path(), ["switch", "--quiet", "main"]);
    let unattached = status();
    assert_eq!(
        unattached["result"]["local"]["comparison_ref"],
        "refs/remotes/scope/main"
    );
    assert_eq!(unattached["result"]["local"]["unpushed_commits"], 0);

    run_git(dir.path(), ["switch", "--quiet", "-c", "normal-request"]);
    for (key, value) in [
        ("scopeRequestId", "req_one"),
        ("scopeRequestOwner", "owner"),
        ("scopeRequestRepo", "repo"),
        ("scopeRequestRemote", "scope"),
        ("scopeRequestName", "fix-one"),
        ("remote", "scope"),
        ("merge", "refs/heads/fix-one"),
    ] {
        run_git(
            dir.path(),
            ["config", &format!("branch.normal-request.{key}"), value],
        );
    }
    run_git(
        dir.path(),
        ["update-ref", "refs/remotes/scope/fix-one", "HEAD"],
    );
    let normal = status();
    assert_eq!(
        normal["result"]["local"]["comparison_ref"],
        "refs/remotes/scope/fix-one"
    );
    assert_eq!(normal["result"]["local"]["unpushed_commits"], 0);
}

#[test]
fn request_push_replaces_amended_history_unless_someone_else_pushed() {
    let dir = TempDir::new("request-push-rewrite");
    create_repo_with_head(dir.path());
    let base = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let bare = TempDir::new("request-push-rewrite-bare");
    run_git(bare.path(), ["init", "--bare"]);
    run_git(
        dir.path(),
        [
            "push",
            bare.path().to_str().unwrap(),
            "main",
            "main:refs/heads/fix-one",
        ],
    );
    let mut detail = request();
    detail["base_main_oid"] = base.clone().into();
    detail["head_oid"] = base.clone().into();
    detail["mergeability"]["current_main_oid"] = base.clone().into();
    detail["mergeability"]["request_head_oid"] = base.clone().into();
    let server = FixtureServer::with_request(detail);
    let transport = BareRepoTransport::new(&server, dir.path(), bare.path());
    let push = || {
        transport
            .command(&server, dir.path())
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
            .unwrap()
    };
    let request_head = || git_stdout(bare.path(), ["rev-parse", "refs/heads/fix-one"]);

    run_git(dir.path(), ["switch", "--quiet", "-c", "fix-one"]);
    fs::write(dir.path().join("fix.txt"), "first\n").unwrap();
    run_git(dir.path(), ["add", "fix.txt"]);
    commit_all(dir.path(), "Fix one");
    assert_eq!(success(push())["command"], "request.push");
    assert_eq!(
        request_head(),
        git_stdout(dir.path(), ["rev-parse", "HEAD"])
    );

    fs::write(dir.path().join("fix.txt"), "amended\n").unwrap();
    run_git(dir.path(), ["add", "fix.txt"]);
    run_git(
        dir.path(),
        [
            "-c",
            "user.email=scope@example.test",
            "-c",
            "user.name=Scope Test",
            "commit",
            "--quiet",
            "--amend",
            "--no-edit",
        ],
    );
    let amended = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    assert_eq!(success(push())["command"], "request.push");
    assert_eq!(request_head(), amended);

    let other = TempDir::new("request-push-rewrite-other");
    run_git(
        other.path(),
        [
            "clone",
            "--quiet",
            "--branch",
            "fix-one",
            bare.path().to_str().unwrap(),
            ".",
        ],
    );
    fs::write(other.path().join("other.txt"), "someone else\n").unwrap();
    run_git(other.path(), ["add", "other.txt"]);
    commit_all(other.path(), "Someone else's change");
    run_git(other.path(), ["push", "--quiet", "origin", "fix-one"]);
    let someone_else = git_stdout(other.path(), ["rev-parse", "HEAD"]);

    run_git(
        dir.path(),
        [
            "-c",
            "user.email=scope@example.test",
            "-c",
            "user.name=Scope Test",
            "commit",
            "--quiet",
            "--amend",
            "-m",
            "Fix one again",
        ],
    );
    let output = push();
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let error: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    assert_eq!(
        error["message"],
        "Someone else updated this request. Fetch it and try again."
    );
    assert!(error["recovery"].is_null(), "{error}");
    assert_eq!(request_head(), someone_else);

    run_git(
        dir.path(),
        [
            "fetch",
            "--quiet",
            bare.path().to_str().unwrap(),
            "+refs/heads/fix-one:refs/remotes/scope/fix-one",
        ],
    );
    assert_eq!(push().status.code(), Some(5));
    assert_eq!(request_head(), someone_else);

    run_git(dir.path(), ["reset", "--quiet", "--hard", &someone_else]);
    fs::write(dir.path().join("fix.txt"), "rebased\n").unwrap();
    run_git(dir.path(), ["add", "fix.txt"]);
    commit_all(dir.path(), "Fix one on top");
    run_git(
        dir.path(),
        [
            "-c",
            "user.email=scope@example.test",
            "-c",
            "user.name=Scope Test",
            "commit",
            "--quiet",
            "--amend",
            "-m",
            "Fix one, amended on top",
        ],
    );
    assert_eq!(success(push())["command"], "request.push");
    assert_eq!(
        request_head(),
        git_stdout(dir.path(), ["rev-parse", "HEAD"])
    );
}

#[test]
fn request_push_without_a_seen_head_only_builds_on_the_current_head() {
    let dir = TempDir::new("request-push-unseen");
    create_repo_with_head(dir.path());
    let base = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let bare = TempDir::new("request-push-unseen-bare");
    run_git(bare.path(), ["init", "--bare"]);
    run_git(
        dir.path(),
        [
            "push",
            bare.path().to_str().unwrap(),
            "main",
            "main:refs/heads/fix-one",
        ],
    );
    let other = TempDir::new("request-push-unseen-other");
    run_git(
        other.path(),
        [
            "clone",
            "--quiet",
            "--branch",
            "fix-one",
            bare.path().to_str().unwrap(),
            ".",
        ],
    );
    fs::write(other.path().join("other.txt"), "someone else\n").unwrap();
    run_git(other.path(), ["add", "other.txt"]);
    commit_all(other.path(), "Someone else's change");
    run_git(other.path(), ["push", "--quiet", "origin", "fix-one"]);
    let someone_else = git_stdout(other.path(), ["rev-parse", "HEAD"]);

    let mut detail = request();
    detail["base_main_oid"] = base.clone().into();
    detail["head_oid"] = someone_else.clone().into();
    detail["mergeability"]["current_main_oid"] = base.clone().into();
    detail["mergeability"]["request_head_oid"] = someone_else.clone().into();
    let server = FixtureServer::with_request(detail);
    let transport = BareRepoTransport::new(&server, dir.path(), bare.path());
    run_git(dir.path(), ["switch", "--quiet", "-c", "fix-one"]);
    fs::write(dir.path().join("fix.txt"), "mine\n").unwrap();
    run_git(dir.path(), ["add", "fix.txt"]);
    commit_all(dir.path(), "My change");

    let output = transport
        .command(&server, dir.path())
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
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let error: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    assert_eq!(
        error["message"],
        "Someone else updated this request. Fetch it and try again."
    );
    assert_eq!(
        git_stdout(bare.path(), ["rev-parse", "refs/heads/fix-one"]),
        someone_else
    );
}
