use super::{LocalState, VisibilityState};
use crate::{
    git_repo::{self, GitRepo},
    repo_config,
};
use anyhow::Context;
use std::process::Command;

pub(super) fn compare_scope_ref(local: &mut LocalState, repo: &GitRepo, remote: &str) {
    if remote.is_empty() {
        return;
    }
    let tracked_request = local.branch.as_deref().and_then(|branch| {
        let request_id = git_repo::branch_config_value(repo, branch, "scopeRequestId")
            .ok()
            .flatten()?;
        let tracked_remote = git_repo::branch_config_value(repo, branch, "remote")
            .ok()
            .flatten()?;
        let merge = git_repo::branch_config_value(repo, branch, "merge")
            .ok()
            .flatten()?;
        (!request_id.is_empty() && tracked_remote == remote)
            .then(|| merge.strip_prefix("refs/heads/").map(str::to_string))
            .flatten()
    });
    let branch = tracked_request.as_deref().unwrap_or("main");
    let reference = format!("refs/remotes/{remote}/{branch}");
    local.unpushed_commits = git_text(
        repo,
        &["rev-list", "--count", &format!("{reference}..HEAD")],
    )
    .and_then(|count| count.parse().ok());
    local.comparison_ref = Some(reference);
}

pub(super) fn local_state(repo: &GitRepo) -> anyhow::Result<LocalState> {
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args(["status", "--porcelain", "-z"])
        .output()
        .context("inspect working tree")?;
    if !output.status.success() {
        anyhow::bail!("Git could not inspect the working tree");
    }
    let upstream = git_text(
        repo,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    Ok(LocalState {
        root: repo.root.clone(),
        branch: git_repo::current_branch(repo).ok(),
        head_oid: git_text(repo, &["rev-parse", "--verify", "HEAD"]),
        dirty: !output.stdout.is_empty(),
        upstream,
        comparison_ref: None,
        unpushed_commits: None,
        visibility: None,
    })
}

pub(super) fn local_visibility(repo: &GitRepo) -> anyhow::Result<VisibilityState> {
    let config = repo_config::load_worktree_scope_repo_config(&repo.root)?;
    let local_hash = scope_domain::repo_config::repo_config_fingerprint(&config)?;
    let base_hash = repo_config::load_worktree_scope_repo_config_base_hash(&repo.root)?;
    let local_edits = base_hash != local_hash;
    Ok(VisibilityState {
        path: repo_config::repo_config_path(&repo.root)?,
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

pub(super) fn git_version_supported(version: &str) -> bool {
    let Some(number) = version.split_whitespace().nth(2) else {
        return false;
    };
    let mut parts = number
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    matches!((parts.next(), parts.next()), (Some(major), Some(minor)) if (major,minor) >= (2,38))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn git_version_support_handles_native_suffixes() {
        for supported in [
            "git version 2.38.0",
            "git version 2.48.1.windows.1",
            "git version 2.39.5 (Apple Git-154)",
        ] {
            assert!(git_version_supported(supported));
        }
        assert!(!git_version_supported("git version 2.37.4"));
    }
}
