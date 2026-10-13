use super::transport::{
    GIT_STALE_LEASE_STATUS, finish_git_plan_output, git_auth_plan, git_command,
    inherited_git_config_count, run_git_plan_output,
};
use super::*;
use crate::error::GitRetrySafety;

pub fn push_head_to_ref_with_headers(
    destination: &str,
    commit_oid: &str,
    refname: &str,
    expected_oid: &str,
    bearer_token: &str,
    headers: &[String],
) -> anyhow::Result<()> {
    let mut all_headers = vec![format!("Authorization: Bearer {bearer_token}")];
    all_headers.extend_from_slice(headers);
    let plan = git_auth_plan(
        vec![
            "-c".into(),
            "push.recurseSubmodules=no".into(),
            "push".into(),
            format!("--force-with-lease={refname}:{expected_oid}"),
            destination.into(),
            format!("{commit_oid}:{refname}"),
        ],
        destination,
        &all_headers,
        inherited_git_config_count(),
    );
    let output = git_command(plan, None)
        .output()
        .context("publish Scope request base and head")?;
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
