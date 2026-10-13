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
    extra_headers: &[String],
) -> anyhow::Result<()> {
    let plan = git_push_ref_auth_plan(
        destination,
        commit_oid,
        refname,
        expected_oid,
        bearer_token,
        extra_headers,
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
    failure: &str,
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
        failure,
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

pub fn fetch_scope_parent_with_bearer(
    repo: &GitRepo,
    destination: &str,
    request_name: &str,
    request_id: &str,
    bearer_token: &str,
) -> anyhow::Result<String> {
    let parent_ref = format!("refs/scope/parents/{request_id}");
    let plan = git_auth_plan(
        vec![
            "-c".into(),
            "protocol.version=2".into(),
            "fetch".into(),
            "--no-tags".into(),
            destination.into(),
            format!("+refs/heads/{request_name}:{parent_ref}"),
        ],
        destination,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        Some(&repo.root),
        "fetch pinned Scope parent",
        "fetch Scope parent failed",
        GitRetrySafety::Idempotent,
    )?;
    let output = git_output_in_repo(repo, &["rev-parse", "--verify", &parent_ref])?;
    if !output.status.success() {
        anyhow::bail!("parent fetch did not produce its ref");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

pub fn fetch_scope_remote_with_bearer_cancellable(
    repo: &GitRepo,
    destination: &str,
    remote: &str,
    branch: &str,
    bearer_token: &str,
    cancellation: &CancellationToken,
    failure: &str,
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
    finish_git_plan_output(output, failure, GitRetrySafety::Idempotent)
}

pub fn git_push_ref_auth_plan(
    destination: &str,
    commit_oid: &str,
    refname: &str,
    expected_oid: &str,
    bearer_token: &str,
    extra_headers: &[String],
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    let mut headers = vec![format!("Authorization: Bearer {bearer_token}")];
    headers.extend_from_slice(extra_headers);
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
        &headers,
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
mod tests;
