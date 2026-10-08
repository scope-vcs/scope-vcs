use crate::{error::ApiError, runtime_budgets::RuntimeBudgets};
use scope_git_process::{
    ProcessLimits, STDERR_DIAGNOSTIC_BYTES, run as run_process, truncated_stderr,
};
use std::{
    path::Path as FsPath,
    process::{Command, ExitStatus, Output},
    time::{Duration, Instant},
};

pub(crate) fn git_process_output(
    command: &mut Command,
    stdin: Option<Vec<u8>>,
    limits: ProcessLimits,
) -> Result<Output, ApiError> {
    let span = prepare_git_subprocess(command)?;
    let _entered = span.enter();
    run_process(command, stdin, limits, "Git command")
        .inspect(|output| record_git_exit(&span, output.status))
        .map_err(|error| {
            if error.is_stdout_limit() {
                ApiError::payload_too_large(error.to_string())
            } else {
                ApiError::infrastructure_unavailable(error.to_string())
            }
        })
}

pub(crate) fn prepare_git_subprocess(command: &mut Command) -> Result<tracing::Span, ApiError> {
    let configured_count = match command
        .get_envs()
        .find(|(key, _)| *key == "GIT_CONFIG_COUNT")
    {
        Some((_, value)) => value.map(std::ffi::OsString::from),
        None => std::env::var_os("GIT_CONFIG_COUNT"),
    };
    let count = match configured_count {
        Some(value) if value.is_empty() => 0,
        Some(value) => value
            .to_str()
            .and_then(|value| {
                value
                    .trim_start_matches(|ch: char| ch.is_ascii_whitespace())
                    .parse::<i64>()
                    .ok()
            })
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| ApiError::infrastructure_unavailable("invalid Git config count"))?,
        None => 0,
    };
    let next_count = count.checked_add(2).ok_or_else(|| {
        ApiError::infrastructure_unavailable("Git config count exceeds supported range")
    })?;
    for (index, (key, value)) in [("maintenance.auto", "false"), ("gc.auto", "0")]
        .into_iter()
        .enumerate()
    {
        let index = count + index as u32;
        command
            .env(format!("GIT_CONFIG_KEY_{index}"), key)
            .env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    command.env("GIT_CONFIG_COUNT", next_count.to_string());
    let subcommand = git_subcommand(command);
    Ok(tracing::info_span!(
        "git subprocess",
        otel.name = %format!("git {subcommand}"),
        otel.kind = "internal",
        git.subcommand = subcommand,
        process.exit.code = tracing::field::Empty,
    ))
}

pub(crate) fn record_git_exit(span: &tracing::Span, status: ExitStatus) {
    if let Some(code) = status.code() {
        span.record("process.exit.code", code);
    }
}

fn git_subcommand(command: &Command) -> &'static str {
    const GIT_SUBCOMMANDS: [&str; 44] = [
        "add",
        "apply",
        "branch",
        "bundle",
        "cat-file",
        "check-ref-format",
        "checkout",
        "clone",
        "commit",
        "commit-tree",
        "config",
        "diff",
        "diff-tree",
        "fetch",
        "for-each-ref",
        "fsck",
        "hash-object",
        "http-backend",
        "index-pack",
        "init",
        "log",
        "ls-remote",
        "ls-tree",
        "merge",
        "merge-base",
        "merge-tree",
        "mktree",
        "pack-objects",
        "push",
        "read-tree",
        "rebase",
        "receive-pack",
        "reset",
        "rev-list",
        "rev-parse",
        "show",
        "show-ref",
        "symbolic-ref",
        "tag",
        "update-index",
        "update-ref",
        "upload-pack",
        "verify-pack",
        "write-tree",
    ];
    let mut args = command.get_args();
    while let Some(arg) = args.next().and_then(|value| value.to_str()) {
        if matches!(
            arg,
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--config-env"
        ) {
            args.next();
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        return GIT_SUBCOMMANDS
            .iter()
            .copied()
            .find(|name| *name == arg)
            .unwrap_or("other");
    }
    "other"
}

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

pub(crate) fn remaining_git_time(deadline: Instant) -> Result<Duration, ApiError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| ApiError::infrastructure_unavailable("Git inspection deadline exceeded"))
}

pub(crate) fn run_git_output_until(
    repo: Option<&FsPath>,
    args: &[&str],
    action: &str,
    deadline: Instant,
) -> Result<Output, ApiError> {
    git_process_output(
        &mut git_repo_command(repo, args),
        None,
        ProcessLimits::new(remaining_git_time(deadline)?),
    )
    .map_err(|error| {
        ApiError::infrastructure_unavailable(format!(
            "failed {action}: {}",
            error.operator_diagnostic()
        ))
    })
}

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

pub(crate) fn git_stdout_text(
    repo: &FsPath,
    args: &[&str],
    action: &str,
) -> Result<String, ApiError> {
    let output = successful_git_output(run_git_output(Some(repo), args, action)?, action)?;
    String::from_utf8(output.stdout).map_err(ApiError::bad_request)
}

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

    #[cfg(unix)]
    #[test]
    fn managed_git_commands_do_not_start_automatic_writers() {
        use std::os::unix::fs::PermissionsExt;

        let repo = temp_repo("automatic-writers");
        let head = commit(&repo, "first");
        for (key, value) in [
            ("gc.auto", "1"),
            ("gc.autoPackLimit", "1"),
            ("gc.autoDetach", "false"),
            ("maintenance.incremental-repack.enabled", "true"),
            ("maintenance.incremental-repack.auto", "1"),
            ("maintenance.autoDetach", "false"),
        ] {
            run_git(
                Some(&repo),
                &["config", key, value],
                "configure maintenance",
            )
            .unwrap();
        }
        for contents in [b"first pack".as_slice(), b"second pack".as_slice()] {
            let oid = git_command_output(
                Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(["hash-object", "-w", "--stdin"]),
                Some(contents),
            )
            .unwrap();
            git_command_output(
                Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .arg("pack-objects")
                    .arg(repo.join(".git/objects/pack/pack")),
                Some(&oid),
            )
            .unwrap();
        }
        let marker = repo.join("gc-triggered");
        let hook = repo.join(".git/hooks/pre-auto-gc");
        fs::write(&hook, "#!/bin/sh\nprintf eligible > \"$SCOPE_GC_MARKER\"\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        let configured_command = || {
            let mut command = Command::new("git");
            command
                .arg("-C")
                .arg(&repo)
                .env("GIT_CONFIG_COUNT", "2")
                .env("GIT_CONFIG_KEY_0", "scope.identity")
                .env("GIT_CONFIG_VALUE_0", "preserved caller value")
                .env("GIT_CONFIG_KEY_1", "gc.autoDetach")
                .env("GIT_CONFIG_VALUE_1", "false")
                .env("SCOPE_GC_MARKER", &marker);
            command
        };
        let identity = git_command_output(
            configured_command().args(["config", "--get", "scope.identity"]),
            None,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(identity).unwrap().trim(),
            "preserved caller value"
        );

        git_command_output(configured_command().args(["gc", "--auto"]), None).unwrap();
        assert!(
            !marker.exists(),
            "managed Git launched automatic garbage collection"
        );
        for count in [Some(""), Some(" 0"), Some("-0"), None] {
            let mut command = configured_command();
            match count {
                Some(value) => command.env("GIT_CONFIG_COUNT", value),
                None => command.env_remove("GIT_CONFIG_COUNT"),
            };
            git_command_output(command.args(["gc", "--auto"]), None).unwrap();
            assert!(
                !marker.exists(),
                "managed Git launched automatic garbage collection"
            );
        }
        git_command_output(
            configured_command().args(["fetch", ".", "refs/heads/main:refs/heads/copied"]),
            None,
        )
        .unwrap();
        assert!(
            !repo.join(".git/objects/pack/multi-pack-index").exists(),
            "managed Git launched automatic maintenance"
        );
        assert_eq!(
            git_stdout_text(
                &repo,
                &["rev-parse", "refs/heads/copied"],
                "read fetched head"
            )
            .unwrap()
            .trim(),
            head
        );
        fs::remove_dir_all(&repo).unwrap();
        assert!(!repo.exists());
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
