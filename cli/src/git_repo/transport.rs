use super::*;
use crate::error::GitRetrySafety;
use crate::progress::{CancellationToken, run_cancellable};
use std::time::Duration;

const GIT_OPERATION_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_GIT_STDOUT_BYTES: usize = 16 * 1024 * 1024;
const GIT_STALE_LEASE_STATUS: &str = "(stale info)";

pub fn push_head_with_bearer(
    destination: &str,
    commit_oid: &str,
    branch: &str,
    bearer_token: &str,
    push_intent_token: &str,
    retry_safety: GitRetrySafety,
) -> anyhow::Result<()> {
    let plan = git_push_auth_plan(
        destination,
        commit_oid,
        branch,
        bearer_token,
        push_intent_token,
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        None,
        "run authenticated Scope git push",
        "git push to Scope failed",
        retry_safety,
    )
}

#[derive(Debug)]
pub struct StaleRefLease;

impl std::fmt::Display for StaleRefLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the remote ref no longer holds the expected commit")
    }
}

impl std::error::Error for StaleRefLease {}

pub fn push_head_to_ref_with_bearer(
    destination: &str,
    commit_oid: &str,
    refname: &str,
    expected_oid: &str,
    bearer_token: &str,
) -> anyhow::Result<()> {
    let plan = git_push_ref_auth_plan(
        destination,
        commit_oid,
        refname,
        expected_oid,
        bearer_token,
        inherited_git_config_count(),
    );
    let output = git_command(plan, None)
        .output()
        .context("run authenticated Scope request branch push")?;
    if !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains(GIT_STALE_LEASE_STATUS)
    {
        return Err(StaleRefLease.into());
    }
    finish_git_plan_output(
        output,
        "git push to Scope request ref failed",
        GitRetrySafety::Idempotent,
    )
}

pub fn clone_with_bearer(
    remote_url: &str,
    bearer_token: &str,
    destination: Option<&Path>,
) -> anyhow::Result<()> {
    let plan = git_clone_auth_plan(
        remote_url,
        bearer_token,
        destination,
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        None,
        "run authenticated Scope git clone",
        "git clone from Scope failed",
        GitRetrySafety::Idempotent,
    )
}

pub fn fetch_scope_remote_with_bearer(
    repo: &GitRepo,
    destination: &str,
    remote: &str,
    branch: &str,
    bearer_token: &str,
) -> anyhow::Result<()> {
    let plan = git_fetch_auth_plan(
        destination,
        remote,
        branch,
        bearer_token,
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        Some(&repo.root),
        "refresh Scope Git remote before push review",
        "refresh Scope Git remote before push review failed; the request was not changed",
        GitRetrySafety::Idempotent,
    )
}

pub fn fetch_scope_remote_refs_with_bearer(
    repo: &GitRepo,
    destination: &str,
    remote: &str,
    bearer_token: &str,
) -> anyhow::Result<()> {
    let plan = git_fetch_refs_auth_plan(
        destination,
        remote,
        bearer_token,
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        Some(&repo.root),
        "fetch Scope refs",
        "git fetch from Scope failed",
        GitRetrySafety::Idempotent,
    )
}

pub fn fetch_scope_remote_with_bearer_cancellable(
    repo: &GitRepo,
    destination: &str,
    remote: &str,
    branch: &str,
    bearer_token: &str,
    cancellation: &CancellationToken,
) -> anyhow::Result<()> {
    let plan = git_fetch_auth_plan(
        destination,
        remote,
        branch,
        bearer_token,
        inherited_git_config_count(),
    );
    let mut command = git_command(plan, Some(&repo.root));
    let output = run_cancellable(
        &mut command,
        None,
        cancellation,
        GIT_OPERATION_TIMEOUT,
        MAX_GIT_STDOUT_BYTES,
    )
    .context("refresh Scope Git remote before push review")?;
    finish_git_plan_output(
        output,
        "refresh Scope Git remote before push review failed; the request was not changed",
        GitRetrySafety::Idempotent,
    )
}

pub fn git_push_ref_auth_plan(
    destination: &str,
    commit_oid: &str,
    refname: &str,
    expected_oid: &str,
    bearer_token: &str,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    git_auth_plan(
        vec![
            "-c".to_string(),
            "push.recurseSubmodules=no".to_string(),
            "push".to_string(),
            format!("--force-with-lease={refname}:{expected_oid}"),
            destination.to_string(),
            format!("{commit_oid}:{refname}"),
        ],
        destination,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_config_count,
    )
}

pub fn git_clone_auth_plan(
    remote_url: &str,
    bearer_token: &str,
    destination: Option<&Path>,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    let mut args = vec!["clone".to_string(), remote_url.to_string()];
    if let Some(destination) = destination {
        args.push(destination.to_string_lossy().to_string());
    }
    git_auth_plan(
        args,
        remote_url,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_config_count,
    )
}

pub fn git_push_auth_plan(
    destination: &str,
    commit_oid: &str,
    branch: &str,
    bearer_token: &str,
    push_intent_token: &str,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    git_auth_plan(
        vec![
            "-c".to_string(),
            "push.recurseSubmodules=no".to_string(),
            "push".to_string(),
            destination.to_string(),
            format!("{commit_oid}:refs/heads/{branch}"),
        ],
        destination,
        &[
            format!("Authorization: Bearer {bearer_token}"),
            format!("X-Scope-Push-Intent: {push_intent_token}"),
        ],
        inherited_config_count,
    )
}

pub fn git_fetch_auth_plan(
    destination: &str,
    remote: &str,
    branch: &str,
    bearer_token: &str,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    git_auth_plan(
        vec![
            "-c".to_string(),
            "protocol.version=2".to_string(),
            "fetch".to_string(),
            "--no-tags".to_string(),
            destination.to_string(),
            format!("+refs/heads/{branch}:refs/remotes/{remote}/{branch}"),
        ],
        destination,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_config_count,
    )
}

pub fn git_fetch_refs_auth_plan(
    destination: &str,
    remote: &str,
    bearer_token: &str,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    git_auth_plan(
        vec![
            "-c".to_string(),
            "protocol.version=2".to_string(),
            "fetch".to_string(),
            "--prune".to_string(),
            remote.to_string(),
        ],
        destination,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_config_count,
    )
}

fn git_auth_plan(
    args: Vec<String>,
    destination: &str,
    headers: &[String],
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    let first_index = inherited_config_count.unwrap_or(0);
    let mut env = vec![(
        "GIT_CONFIG_COUNT".to_string(),
        (first_index + headers.len()).to_string(),
    )];
    for (offset, header) in headers.iter().enumerate() {
        let index = first_index + offset;
        env.push((
            format!("GIT_CONFIG_KEY_{index}"),
            format!("http.{destination}.extraHeader"),
        ));
        env.push((format!("GIT_CONFIG_VALUE_{index}"), header.clone()));
    }
    GitCommandPlan { args, env }
}

fn inherited_git_config_count() -> Option<usize> {
    env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|value| value.parse().ok())
}

fn git_command(plan: GitCommandPlan, cwd: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.env("LC_ALL", "C");
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.args(plan.args);
    command.envs(plan.env);
    command
}

fn run_git_plan_output(
    plan: GitCommandPlan,
    cwd: Option<&Path>,
    context: &str,
    failure: &str,
    retry_safety: GitRetrySafety,
) -> anyhow::Result<()> {
    let output = git_command(plan, cwd)
        .output()
        .with_context(|| context.to_string())?;
    finish_git_plan_output(output, failure, retry_safety)
}

fn finish_git_plan_output(
    output: Output,
    failure: &str,
    retry_safety: GitRetrySafety,
) -> anyhow::Result<()> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(crate::error::CliError::git_failure(failure, stderr, retry_safety).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::GitRetrySafety;
    use crate::error::{ExitCategory, exit_code, json_response};
    use scope_api_contract::ErrorCode;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

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
        fetch_scope_remote_with_bearer(&repo, destination, "scope", "main", "fixture-token")
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
        for refname in ["refs/heads/main", "refs/heads/fix-fixture"] {
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
                    git_push_ref_auth_plan(destination, &head, refname, "", "fixture-token", None)
                }
            };
            let first = git_command(plan(), Some(&repo.root)).output().unwrap();
            assert!(first.status.success(), "{:?}", first);
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
    fn refused_git_fetch_is_retryable_and_does_not_change_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let error = fetch_error(&format!("http://{address}/repo"));
        let response = json_response(&error).error;
        assert_eq!(response.code, ErrorCode::ServiceUnavailable);
        assert!(response.retryable);
        assert!(response.message.contains("the request was not changed"));
        assert_eq!(exit_code(&error), ExitCategory::Temporary as u8);
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
            for push_safety in [
                None,
                Some(GitRetrySafety::Idempotent),
                Some(GitRetrySafety::MayCreateRequest),
            ] {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let address = listener.local_addr().unwrap();
                let server = thread::spawn(move || {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        let count = socket.read(&mut buffer).unwrap();
                        assert!(count > 0);
                        request.extend_from_slice(&buffer[..count]);
                    }
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
}
