//! Git process execution and output decoding for API callers.

use crate::{error::ApiError, runtime_budgets::RuntimeBudgets};
use scope_git_process::{
    ProcessLimits, STDERR_DIAGNOSTIC_BYTES, run as run_process, truncated_stderr,
};
use std::{
    path::Path as FsPath,
    process::{Command, Output},
    time::Duration,
};

/// Runs a prepared command and maps process-level failures (spawn, I/O, timeout, stdout
/// budget) onto API errors. A non-zero exit is returned to the caller as a normal `Output`.
pub(crate) fn git_process_output(
    command: &mut Command,
    stdin: Option<Vec<u8>>,
    limits: ProcessLimits,
) -> Result<Output, ApiError> {
    run_process(command, stdin, limits, "Git command").map_err(|error| {
        if error.is_stdout_limit() {
            ApiError::payload_too_large(error.to_string())
        } else {
            ApiError::infrastructure_unavailable(error.to_string())
        }
    })
}

/// Runs a prepared command and returns stdout, treating any non-zero exit as an
/// infrastructure failure described by the (bounded) stderr.
pub(crate) fn git_command_output_with_timeout(
    command: &mut Command,
    stdin: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Vec<u8>, ApiError> {
    let output = git_process_output(command, stdin, ProcessLimits::new(timeout))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(ApiError::infrastructure_unavailable(
        truncated_git_stderr(&output.stderr).trim(),
    ))
}

pub(crate) fn git_command_output(
    command: &mut Command,
    stdin: Option<&[u8]>,
) -> Result<Vec<u8>, ApiError> {
    git_command_output_with_timeout(
        command,
        stdin.map(Vec::from),
        RuntimeBudgets::default_git_command_timeout(),
    )
}

pub(crate) fn truncated_git_stderr(stderr: &[u8]) -> String {
    truncated_stderr(stderr, STDERR_DIAGNOSTIC_BYTES)
}

fn git_repo_command(repo: Option<&FsPath>, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    if let Some(repo) = repo {
        command.arg("-C").arg(repo);
    }
    command.args(args);
    command
}

/// Runs `git <args>` (optionally inside `repo`) and returns the raw output. Only process
/// failures are errors; callers that need a successful exit use `run_git` or
/// `git_stdout_text`.
pub(crate) fn run_git_output(
    repo: Option<&FsPath>,
    args: &[&str],
    action: &str,
) -> Result<Output, ApiError> {
    git_process_output(
        &mut git_repo_command(repo, args),
        None,
        ProcessLimits::new(RuntimeBudgets::default_git_command_timeout()),
    )
    .map_err(|error| {
        ApiError::infrastructure_unavailable(format!(
            "failed {action}: {}",
            error.operator_diagnostic()
        ))
    })
}

/// Like `run_git_output`, but rejects stdout larger than `max_stdout_bytes` with a
/// payload-too-large error naming the action.
pub(crate) fn run_git_output_bounded(
    repo: Option<&FsPath>,
    args: &[&str],
    action: &str,
    max_stdout_bytes: usize,
) -> Result<Output, ApiError> {
    git_process_output(
        &mut git_repo_command(repo, args),
        None,
        ProcessLimits::new(RuntimeBudgets::default_git_command_timeout())
            .with_max_stdout_bytes(max_stdout_bytes),
    )
    .map_err(|error| match error.status() {
        axum::http::StatusCode::PAYLOAD_TOO_LARGE => {
            ApiError::payload_too_large(format!("{action} exceeded {max_stdout_bytes} bytes"))
        }
        _ => error,
    })
}

/// Turns a non-zero exit into an infrastructure error carrying the action and stderr.
pub(crate) fn successful_git_output(output: Output, action: &str) -> Result<Output, ApiError> {
    if output.status.success() {
        return Ok(output);
    }
    Err(ApiError::infrastructure_unavailable(format!(
        "{action}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

pub(crate) fn run_git(repo: Option<&FsPath>, args: &[&str], action: &str) -> Result<(), ApiError> {
    successful_git_output(run_git_output(repo, args, action)?, action).map(drop)
}

/// Runs `git <args>` inside `repo` and returns its stdout as text, untrimmed.
pub(crate) fn git_stdout_text(
    repo: &FsPath,
    args: &[&str],
    action: &str,
) -> Result<String, ApiError> {
    let output = successful_git_output(run_git_output(Some(repo), args, action)?, action)?;
    String::from_utf8(output.stdout).map_err(ApiError::bad_request)
}

/// `git merge-base --is-ancestor`: exit 0 means ancestor, exit 1 means not an ancestor,
/// anything else (unknown object, corrupt repository) is an infrastructure error and
/// never a user-facing "not an ancestor" answer.
pub(crate) fn git_is_ancestor(
    repo: &FsPath,
    ancestor: &str,
    descendant: &str,
    action: &str,
) -> Result<bool, ApiError> {
    let output = run_git_output(
        Some(repo),
        &["merge-base", "--is-ancestor", ancestor, descendant],
        action,
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(ApiError::infrastructure_unavailable(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

/// Lists `(refname, object id)` pairs under the given prefixes (all refs when empty).
pub(crate) fn git_ref_listing(
    repo: &FsPath,
    prefixes: &[&str],
    action: &str,
) -> Result<Vec<(String, String)>, ApiError> {
    let mut args = vec!["for-each-ref", "--format=%(refname)%00%(objectname)"];
    args.extend(prefixes.iter().copied());
    let text = git_stdout_text(repo, &args, action)?;
    text.lines()
        .map(|line| {
            let (refname, oid) = line
                .split_once('\0')
                .ok_or_else(|| ApiError::internal_message("invalid git ref listing"))?;
            Ok((refname.to_string(), oid.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EMPTY_GIT_OID;
    use axum::http::StatusCode;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn stderr_truncation_preserves_utf8_boundaries() {
        let stderr = "é".repeat(STDERR_DIAGNOSTIC_BYTES);

        let truncated = truncated_git_stderr(stderr.as_bytes());

        assert!(truncated.ends_with("..."));
        assert!(truncated.is_char_boundary(truncated.len() - 3));
    }

    #[test]
    fn bounded_git_output_maps_size_limit_to_payload_too_large() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("printf 12345");

        let error = git_process_output(
            &mut command,
            None,
            ProcessLimits::new(Duration::from_secs(1)).with_max_stdout_bytes(4),
        )
        .unwrap_err();

        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(
            error
                .operator_diagnostic()
                .contains("stdout exceeded 4 bytes")
        );
    }

    #[test]
    fn bounded_git_output_names_the_action_when_stdout_is_too_large() {
        let error =
            run_git_output_bounded(None, &["--version"], "reading Git version", 1).unwrap_err();
        assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(
            error
                .operator_diagnostic()
                .contains("reading Git version exceeded 1 bytes")
        );
    }

    #[test]
    fn ancestry_distinguishes_not_an_ancestor_from_git_failures() {
        let repo = temp_repo("ancestry");
        let first = commit(&repo, "first");
        let second = commit(&repo, "second");

        assert!(git_is_ancestor(&repo, &first, &second, "checking ancestry").unwrap());
        assert!(!git_is_ancestor(&repo, &second, &first, "checking ancestry").unwrap());

        let error =
            git_is_ancestor(&repo, EMPTY_GIT_OID, &second, "checking ancestry").unwrap_err();
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            error
                .operator_diagnostic()
                .starts_with("checking ancestry:")
        );
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn ref_listing_parses_every_ref_under_the_requested_prefixes() {
        let repo = temp_repo("ref-listing");
        let head = commit(&repo, "first");
        run_git(Some(&repo), &["tag", "v1"], "tag").unwrap();

        let heads = git_ref_listing(&repo, &["refs/heads"], "listing heads").unwrap();
        assert_eq!(heads, vec![("refs/heads/main".to_string(), head.clone())]);
        let all = git_ref_listing(&repo, &[], "listing refs").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].0, "refs/tags/v1");
        let _ = fs::remove_dir_all(repo);
    }

    fn temp_repo(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "scope-git-command-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        run_git(Some(&path), &["init", "-q", "-b", "main"], "init").unwrap();
        path
    }

    fn commit(repo: &FsPath, name: &str) -> String {
        fs::write(repo.join(name), name).unwrap();
        run_git(Some(repo), &["add", "."], "add").unwrap();
        run_git(
            Some(repo),
            &[
                "-c",
                "user.name=Scope Tests",
                "-c",
                "user.email=scope@example.com",
                "commit",
                "-qm",
                name,
            ],
            "commit",
        )
        .unwrap();
        git_stdout_text(repo, &["rev-parse", "HEAD"], "head")
            .unwrap()
            .trim()
            .to_string()
    }
}
