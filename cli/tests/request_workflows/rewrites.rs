use super::*;

#[test]
fn request_push_replaces_amended_history_unless_someone_else_pushed() {
    let dir = TempDir::new("request-push-rewrite");
    create_repo_with_head(dir.path());
    let base = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let bare = TempDir::new("request-push-rewrite-bare");
    run_git(bare.path(), ["init", "--bare"]);
    // Scope advertises a new request's branch at the base the request started from.
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
}
