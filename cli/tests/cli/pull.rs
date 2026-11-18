use crate::support::*;
use axum::{Json, Router, routing::get};
use scope_cli::repo_config::{
    load_worktree_scope_repo_config, load_worktree_scope_repo_config_base_hash, repo_config_path,
};
use serde_json::{Value, json};
use std::fs;

#[test]
fn pull_adopts_the_advertised_git_origin_and_a_failed_fetch_keeps_local_state() {
    let dir = TempDir::new("pull-visibility-recovery");
    create_repo_with_head(dir.path());
    let server = TestServer::with_url(|api_url| {
        let advertised = format!("{api_url}/git/public/owner/repo");
        Router::new()
            .route(
                "/v1/session",
                get(|| async {
                    Json(session_response(
                        "user_test",
                        "reader",
                        "reader@example.test",
                    ))
                }),
            )
            .route(
                "/v1/repos/owner/repo",
                get(move || async move {
                    Json(repository_response(json!({ "git_remote_url": advertised })))
                }),
            )
    });
    let retired_git_origin = "http://127.0.0.1:9";
    run_git(
        dir.path(),
        [
            "remote",
            "add",
            "scope",
            &format!("{retired_git_origin}/git/public/owner/repo"),
        ],
    );
    run_git(
        dir.path(),
        ["config", "scope.gitOrigin", retired_git_origin],
    );
    let original_head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let config_path = repo_config_path(dir.path()).unwrap();

    let (first, _) = failed_pull(&server, dir.path());
    assert_eq!(first["recovery"]["visibility_setup"], "created");
    assert_eq!(first["recovery"]["operation"], "pull");
    assert!(
        first["recovery"]["recovery"]
            .as_str()
            .unwrap()
            .contains("rerun scope pull")
    );
    let config = load_worktree_scope_repo_config(dir.path()).unwrap();
    let original_base = load_worktree_scope_repo_config_base_hash(dir.path()).unwrap();
    assert_eq!(git_stdout(dir.path(), ["rev-parse", "HEAD"]), original_head);

    fs::remove_file(config_path.parent().unwrap().join("repo-state.json")).unwrap();
    let (recovered, _) = failed_pull(&server, dir.path());
    assert_eq!(recovered["recovery"]["visibility_setup"], "base_recovered");
    assert_eq!(recovered["error"]["code"], first["error"]["code"]);
    assert_eq!(load_worktree_scope_repo_config(dir.path()).unwrap(), config);
    assert_eq!(
        load_worktree_scope_repo_config_base_hash(dir.path()).unwrap(),
        original_base
    );

    let (retry, _) = failed_pull(&server, dir.path());
    assert!(retry["recovery"].is_null());
    assert_eq!(retry["error"]["code"], first["error"]["code"]);
    assert_eq!(git_stdout(dir.path(), ["rev-parse", "HEAD"]), original_head);
    assert_eq!(
        git_stdout(dir.path(), ["remote", "get-url", "scope"]),
        format!("{}/git/public/owner/repo", server.api_url)
    );
    assert_eq!(
        git_stdout(dir.path(), ["config", "scope.gitOrigin"]),
        server.api_url
    );
}

#[test]
fn pull_moves_other_remotes_off_the_retired_git_origin_only_where_the_move_is_exact() {
    let dir = TempDir::new("pull-remote-migration");
    create_repo_with_head(dir.path());
    let server = TestServer::with_url(|api_url| {
        let advertised = format!("{api_url}/git/public/owner/repo");
        Router::new()
            .route(
                "/v1/session",
                get(|| async {
                    Json(session_response(
                        "user_test",
                        "reader",
                        "reader@example.test",
                    ))
                }),
            )
            .route(
                "/v1/repos/owner/repo",
                get(move || async move {
                    Json(repository_response(json!({ "git_remote_url": advertised })))
                }),
            )
    });
    let old = "http://127.0.0.1:9";
    let new = server.api_url.as_str();
    let included = dir.path().join(".git").join("included-remotes");
    fs::write(
        &included,
        format!("[remote \"included\"]\n\turl = {old}/git/public/owner/included\n"),
    )
    .unwrap();
    let settings = [
        ("url.{old}/.insteadOf", "fetch-old:"),
        ("url.{old}/.pushInsteadOf", "push-old:"),
        ("url.https://mirror.example/.insteadOf", "split:"),
        ("url.{old}/.pushInsteadOf", "split:"),
        ("url.{old}/.insteadOf", "both:"),
        ("url.{old}/.pushInsteadOf", "both:"),
        ("url.{old}/.insteadOf", "fetch-split:"),
        ("url.https://mirror.example/.pushInsteadOf", "fetch-split:"),
        (
            "url.https://rewritten.example/.insteadOf",
            "{new}/git/public/owner/rewritten",
        ),
        ("remote.scope.url", "{old}/git/public/owner/repo"),
        ("remote.direct.url", "{old}/git/public/owner/direct"),
        ("remote.explicit.url", "{old}/git/private/owner/explicit"),
        ("remote.explicit.url", "{old}/git/public/owner/explicit"),
        (
            "remote.explicit.pushurl",
            "{old}/git/private/owner/explicit",
        ),
        ("remote.explicit.pushurl", "{old}/git/public/owner/explicit"),
        (
            "remote.explicit.pushurl",
            "https://mirror.example/owner/explicit.git",
        ),
        ("remote.aliased.url", "fetch-old:git/private/owner/aliased"),
        (
            "remote.aliased.pushurl",
            "fetch-old:git/public/owner/aliased",
        ),
        (
            "remote.fetch-aliased.url",
            "fetch-old:git/public/owner/fetch-aliased",
        ),
        (
            "remote.push-aliased.url",
            "https://mirror.example/owner/push-aliased.git",
        ),
        (
            "remote.push-aliased.url",
            "push-old:git/public/owner/push-aliased",
        ),
        ("remote.split.url", "split:git/private/owner/split"),
        ("remote.both.url", "both:git/public/owner/both"),
        (
            "remote.fetch-split.url",
            "fetch-split:git/public/owner/fetch-split",
        ),
        ("remote.rewritten.url", "{old}/git/public/owner/rewritten"),
        ("include.path", included.to_str().unwrap()),
        ("scope.gitOrigin", old),
    ];
    for (key, value) in settings {
        let fill = |text: &str| text.replace("{old}", old).replace("{new}", new);
        run_git(dir.path(), ["config", "--add", &fill(key), &fill(value)]);
    }

    let (_, stderr) = failed_pull(&server, dir.path());

    for (remote, expected) in [
        ("direct", "url {new}/git/public/owner/direct"),
        (
            "explicit",
            "url {new}/git/private/owner/explicit\n\
             url {new}/git/public/owner/explicit\n\
             pushurl {new}/git/private/owner/explicit\n\
             pushurl {new}/git/public/owner/explicit\n\
             pushurl https://mirror.example/owner/explicit.git",
        ),
        (
            "aliased",
            "url {new}/git/private/owner/aliased\n\
             pushurl {new}/git/public/owner/aliased",
        ),
        ("fetch-aliased", "url {new}/git/public/owner/fetch-aliased"),
        (
            "push-aliased",
            "url https://mirror.example/owner/push-aliased.git\n\
             url push-old:git/public/owner/push-aliased\n\
             pushurl {new}/git/public/owner/push-aliased",
        ),
        (
            "split",
            "url split:git/private/owner/split\n\
             pushurl {new}/git/private/owner/split",
        ),
        (
            "both",
            "url {new}/git/public/owner/both\n\
             pushurl {new}/git/public/owner/both",
        ),
        (
            "fetch-split",
            "url fetch-split:git/public/owner/fetch-split",
        ),
        ("rewritten", "url {old}/git/public/owner/rewritten"),
        ("included", "url {old}/git/public/owner/included"),
    ] {
        let pattern = format!(r"^remote\.{remote}\.(url|pushurl)$");
        let actual = git_stdout(dir.path(), ["config", "--get-regexp", &pattern])
            .replace(&format!("remote.{remote}."), "");
        let expected = expected.replace("{old}", old).replace("{new}", new);
        assert_eq!(actual, expected, "{remote}");
    }
    let left: Vec<_> = stderr
        .lines()
        .filter(|line| line.contains("still uses the retired Scope Git origin"))
        .collect();
    assert_eq!(left.len(), 3, "{stderr}");
    for remote in ["fetch-split", "rewritten", "included"] {
        assert!(
            left.iter()
                .any(|line| line.starts_with(&format!("Remote {remote} "))),
            "{stderr}"
        );
    }
}

fn failed_pull(server: &TestServer, cwd: &std::path::Path) -> (Value, String) {
    let output = server
        .command(cwd)
        .args(["--json", "pull", "--remote", "scope"])
        .output()
        .unwrap();
    assert_failure(&output, "scope pull with unavailable Git endpoint");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let result = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    (result, stderr)
}
