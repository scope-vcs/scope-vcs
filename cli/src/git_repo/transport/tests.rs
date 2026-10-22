use super::*;
use crate::error::GitRetrySafety;
use crate::error::{ExitCategory, exit_code, json_response};
use scope_api_contract::ErrorCode;
use std::{io::Write, net::TcpListener, thread};

fn test_repo() -> (tempfile::TempDir, GitRepo) {
    let directory = tempfile::tempdir().unwrap();
    let repo = GitRepo {
        root: directory.path().to_path_buf(),
    };
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "http.proxy", ""],
        vec!["config", "credential.helper", ""],
    ] {
        assert!(
            Command::new("git")
                .current_dir(&repo.root)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    (directory, repo)
}

fn fetch_error(destination: &str) -> anyhow::Error {
    let (_directory, repo) = test_repo();
    fetch_scope_remote_with_bearer(
        &repo,
        destination,
        "scope",
        "main",
        "fixture-token",
        None,
        "refresh request before publishing failed; the request was not changed",
    )
    .context("refresh request before publishing")
    .unwrap_err()
}

fn commit(repo: &GitRepo) {
    assert!(
        Command::new("git")
            .current_dir(&repo.root)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "fixture"
            ])
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn applied_push_retries_leave_main_and_leased_request_refs_unchanged() {
    let (_directory, repo) = test_repo();
    commit(&repo);
    let remote = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--bare", "--quiet"])
            .arg(remote.path())
            .status()
            .unwrap()
            .success()
    );
    let destination = remote.path().to_str().unwrap();
    let head = head_oid(&repo).unwrap();
    for refname in [
        "refs/heads/main",
        "refs/heads/fix-fixture",
        "refs/heads/existing-request",
    ] {
        let expected_oid = if refname == "refs/heads/existing-request" {
            let initial = git_command(
                git_push_ref_auth_plan(destination, &head, refname, "", "fixture-token", &[], None),
                Some(&repo.root),
            )
            .output()
            .unwrap();
            assert!(initial.status.success(), "{initial:?}");
            run_git_in_repo(
                &repo,
                &["update-ref", "refs/remotes/scope/existing-request", &head],
            )
            .unwrap();
            commit(&repo);
            scope_remote_head_oid(&repo, "scope", "existing-request")
                .unwrap()
                .unwrap()
        } else {
            String::new()
        };
        let head = head_oid(&repo).unwrap();
        let plan = || {
            if refname == "refs/heads/main" {
                git_push_auth_plan(
                    destination,
                    &head,
                    "main",
                    "fixture-token",
                    "fixture-intent",
                    None,
                )
            } else {
                git_push_ref_auth_plan(
                    destination,
                    &head,
                    refname,
                    &expected_oid,
                    "fixture-token",
                    &[],
                    None,
                )
            }
        };
        let first = git_command(plan(), Some(&repo.root)).output().unwrap();
        assert!(first.status.success(), "{:?}", first);
        if !expected_oid.is_empty() {
            assert_ne!(expected_oid, head);
            assert_eq!(
                scope_remote_head_oid(&repo, "scope", "existing-request")
                    .unwrap()
                    .unwrap(),
                expected_oid
            );
        }
        let retry = git_command(plan(), Some(&repo.root)).output().unwrap();
        assert!(retry.status.success(), "{:?}", retry);
        assert!(String::from_utf8_lossy(&retry.stderr).contains("Everything up-to-date"));
        let remote_head = Command::new("git")
            .arg("--git-dir")
            .arg(remote.path())
            .args(["rev-parse", refname])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&remote_head.stdout).trim(), head);
    }
}

#[test]
fn refused_git_fetch_preserves_caller_context_and_retryability() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let destination = format!("http://{address}/repo");
    let failure = "refresh request failed; the request was not changed";
    for cancellable in [false, true] {
        let (_directory, repo) = test_repo();
        let error = if cancellable {
            fetch_scope_remote_with_bearer(
                &repo,
                &destination,
                "scope",
                "main",
                "fixture-token",
                Some(&CancellationToken::new()),
                failure,
            )
        } else {
            fetch_scope_remote_with_bearer(
                &repo,
                &destination,
                "scope",
                "main",
                "fixture-token",
                None,
                failure,
            )
        }
        .unwrap_err();
        let response = json_response(&error).error;
        assert_eq!(response.code, ErrorCode::ServiceUnavailable);
        assert!(response.retryable);
        assert!(
            response.message.starts_with(&format!("{failure}: fatal:")),
            "{}",
            response.message
        );
        assert_eq!(exit_code(&error), ExitCategory::Temporary as u8);
    }
}

#[test]
fn git_http_failures_preserve_response_classification() {
    for (status, code, category, retryable) in [
        (
            Some("401 Unauthorized"),
            ErrorCode::Unauthorized,
            ExitCategory::Authentication,
            false,
        ),
        (
            Some("403 Forbidden"),
            ErrorCode::Forbidden,
            ExitCategory::Policy,
            false,
        ),
        (
            Some("500 Internal Server Error"),
            ErrorCode::Internal,
            ExitCategory::Unexpected,
            false,
        ),
        (
            Some("429 Too Many Requests"),
            ErrorCode::ServiceUnavailable,
            ExitCategory::Temporary,
            true,
        ),
        (
            Some("502 Bad Gateway"),
            ErrorCode::ServiceUnavailable,
            ExitCategory::Temporary,
            true,
        ),
        (
            Some("503 Service Unavailable"),
            ErrorCode::ServiceUnavailable,
            ExitCategory::Temporary,
            true,
        ),
        (
            Some("504 Gateway Timeout"),
            ErrorCode::ServiceUnavailable,
            ExitCategory::Temporary,
            true,
        ),
        (
            None,
            ErrorCode::ServiceUnavailable,
            ExitCategory::Temporary,
            true,
        ),
    ] {
        let push_safeties: &[Option<GitRetrySafety>] =
            if matches!(status, Some("403 Forbidden" | "503 Service Unavailable")) {
                &[
                    None,
                    Some(GitRetrySafety::Idempotent),
                    Some(GitRetrySafety::MayCreateRequest),
                ]
            } else {
                &[None]
            };
        for &push_safety in push_safeties {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                crate::test_support::read_http_request(&mut socket).unwrap();
                if let Some(status) = status {
                    write!(
                        socket,
                        "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                }
            });
            let destination = format!("http://{address}/repo");
            let error = match push_safety {
                None => fetch_error(&destination),
                Some(safety) => {
                    let (_directory, repo) = test_repo();
                    commit(&repo);
                    run_git_plan_output(
                        git_push_auth_plan(
                            &destination,
                            "HEAD",
                            "main",
                            "fixture-token",
                            "fixture-intent",
                            None,
                        ),
                        Some(&repo.root),
                        "push fixture",
                        "git push to Scope failed",
                        safety,
                    )
                    .unwrap_err()
                }
            };
            server.join().unwrap();
            let response = json_response(&error).error;
            if retryable && matches!(push_safety, Some(GitRetrySafety::MayCreateRequest)) {
                assert_eq!(response.code, ErrorCode::Conflict, "{status:?}: {error:#}");
                assert!(!response.retryable);
                assert!(response.message.contains("the push may have been applied"));
                assert!(response.message.contains("scope request list"));
                assert!(response.message.contains("scope request show"));
                assert_eq!(exit_code(&error), ExitCategory::StateConflict as u8);
                continue;
            }
            assert_eq!(response.code, code, "{status:?}: {error:#}");
            assert_eq!(response.retryable, retryable);
            assert_eq!(exit_code(&error), category as u8);
        }
    }
}
