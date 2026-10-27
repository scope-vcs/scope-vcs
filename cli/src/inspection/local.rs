use super::{LocalState, VisibilityState};
use crate::{
    api::RequestSummaryResponse,
    git_repo::{self, GitRepo, GitSnapshot},
    git_transport::ScopeRemote,
    repo_config, request,
};
use anyhow::Context;
use std::process::Command;

pub(super) fn compare_scope_ref(
    local: &mut LocalState,
    repo: &GitRepo,
    checkout: &GitSnapshot,
    target: &ScopeRemote,
    main_remote: &str,
    resolved_request: Option<&RequestSummaryResponse>,
) -> Option<String> {
    if target.remote.is_empty() {
        return None;
    }
    let comparison = local
        .branch
        .as_deref()
        .map(|branch| {
            request::resolve_request_comparison_ref(checkout, branch, target, resolved_request)
        })
        .unwrap_or(request::RequestComparison::Main);
    let reference = match comparison {
        request::RequestComparison::Main => format!("refs/remotes/{main_remote}/main"),
        request::RequestComparison::Request(reference) => reference,
        request::RequestComparison::Unavailable(request_id) => {
            local.comparison_ref = None;
            local.unpushed_commits = None;
            return Some(request_id);
        }
    };
    local.unpushed_commits = git_text(
        repo,
        &["rev-list", "--count", &format!("{reference}..HEAD")],
    )
    .and_then(|count| count.parse().ok());
    local.comparison_ref = Some(reference);
    None
}

pub(super) fn local_state(repo: &GitRepo, checkout: &GitSnapshot) -> anyhow::Result<LocalState> {
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args(["status", "--porcelain=v2", "--branch", "-z"])
        .output()
        .context("inspect working tree")?;
    if !output.status.success() {
        anyhow::bail!("Git could not inspect the working tree");
    }
    let status = String::from_utf8_lossy(&output.stdout);
    let header = |name: &str| {
        status.split('\0').find_map(|entry| {
            entry
                .strip_prefix("# ")?
                .strip_prefix(name)?
                .strip_prefix(' ')
        })
    };
    Ok(LocalState {
        root: repo.root.clone(),
        branch: checkout.branch().map(str::to_string),
        head_oid: header("branch.oid")
            .filter(|oid| *oid != "(initial)")
            .map(str::to_string),
        dirty: status
            .split('\0')
            .any(|entry| !entry.is_empty() && !entry.starts_with("# ")),
        upstream: header("branch.ab")
            .and(header("branch.upstream"))
            .map(str::to_string),
        comparison_ref: None,
        unpushed_commits: None,
        visibility: None,
    })
}

pub(super) fn local_visibility(repo: &GitRepo) -> anyhow::Result<VisibilityState> {
    let synced = repo_config::load_synced_worktree_scope_repo_config(&repo.root)?;
    let local_hash = scope_domain::repo_config::repo_config_fingerprint(&synced.config)?;
    let base_hash = synced.base_hash;
    let local_edits = base_hash != local_hash;
    Ok(VisibilityState {
        path: synced.path,
        local_hash,
        base_hash: Some(base_hash),
        local_edits: Some(local_edits),
        server_hash: None,
        server_changed: None,
    })
}

pub(super) fn git_text(repo: &GitRepo, args: &[&str]) -> Option<String> {
    let output = git_repo::git_output_in_repo(repo, args).ok()?;
    output
        .status
        .success()
        .then(|| {
            String::from_utf8_lossy(&output.stdout)
                .trim_end_matches(['\r', '\n'])
                .to_string()
        })
        .filter(|value| !value.is_empty())
}
