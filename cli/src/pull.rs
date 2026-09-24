use crate::api::ApiSession;
use crate::display::short_oid;
use crate::{
    api::{RepositoryActor, api_url, get_repo, get_repo_config, http_client},
    git_repo::{
        GitRepo, branch_config_value, current_branch, ensure_git_repo_ready, git_remote_fetch_url,
        head_oid, install_scope_fetch_auth, run_git_in_repo, scope_git_origin,
    },
    git_transport::{ScopeRemote, select_scope_fetch_remote},
    login::session_from_cache_or_browser,
    push::DEFAULT_SCOPE_BRANCH,
    repo_config::{
        WorktreeRepoConfigPresence, WorktreeRepoConfigSync, default_scope_repo_config,
        load_worktree_scope_repo_config, load_worktree_scope_repo_config_base_hash,
        sync_missing_worktree_scope_repo_config, worktree_scope_repo_config_presence,
    },
};
use crate::{error::CliError, execution::emit};
use anyhow::{Context, bail};
use serde_json::json;
use std::{collections::BTreeMap, process::Command};

pub fn run(explicit_remote: Option<&str>) -> anyhow::Result<()> {
    let repo = ensure_git_repo_ready("scope pull")?;
    let branch = current_branch(&repo)?;
    let previous_head = head_oid(&repo)?;
    let api_url = api_url()?;
    let remote = select_scope_fetch_remote(&repo, &api_url, explicit_remote)?;
    let git_origin = scope_git_origin(&repo, &api_url)?;
    let target = ScopeRemote::parse(&git_origin, &remote, &git_remote_fetch_url(&repo, &remote)?)?;
    let client = http_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    let visibility_sync = sync_pull_visibility(
        &repo,
        ApiSession::new(&client, &api_url, &session.token),
        &target,
    )?;

    // Persist the permissioned URL and credential helper so plain `git fetch` and
    // `git pull` have exactly the same view after this command returns.
    run_git_in_repo(
        &repo,
        &["remote", "set-url", &remote, &target.permissioned_url],
    )?;
    install_scope_fetch_auth(&repo.root, &target.permissioned_url, &api_url)?;

    let before = remote_refs(&repo, &remote)?;
    run_git_in_repo(&repo, &["fetch", "--prune", &remote])?;
    let after = remote_refs(&repo, &remote)?;
    let mut lines = ref_change_lines(&remote, &before, &after);
    if visibility_sync == WorktreeRepoConfigSync::Created {
        lines.push("Loaded this worktree's visibility config from Scope.".into());
    } else if visibility_sync == WorktreeRepoConfigSync::BaseRecovered {
        lines.push("Recovered this worktree's visibility sync base.".into());
    }
    let upstream = tracked_branch(&repo, &remote, &branch)?;
    let tracked_name = upstream.as_deref().filter(|name| after.contains_key(*name));
    let mut moved = false;
    if let Some(tracked_name) = tracked_name {
        let tracked = format!("refs/remotes/{remote}/{tracked_name}");
        eprintln!(
            "Fast-forward {}/{} local {branch} to {tracked} at {}",
            target.owner, target.repo, after[tracked_name]
        );
        run_git_in_repo(&repo, &["merge", "--ff-only", &tracked]).map_err(|error| CliError::partial(
            format!("Fetched Scope refs, but could not fast-forward {branch}: {error:#}"),
            json!({"operation": "pull", "repository": format!("{}/{}", target.owner, target.repo), "fetched": true, "branch": branch, "previous_head": previous_head, "remote_refs": after, "recovery": "Inspect git status and the local branch divergence. Resolve local changes or divergence, then repeat scope pull; no force reset is needed."})
        ))?;
        moved = head_oid(&repo)? != previous_head;
        lines.push(format!(
            "{branch} is up to date with {remote}/{tracked_name}."
        ));
    } else if let Some(upstream) = upstream {
        lines.push(format!("Fetched every visible Scope ref; upstream {remote}/{upstream} for local branch {branch} is unavailable."));
    } else {
        lines.push(format!("Fetched every visible Scope ref; local branch {branch} does not track a branch on {remote}, so it was not moved."));
    }

    emit(
        "pull",
        &json!({"repository": format!("{}/{}", target.owner, target.repo), "remote": remote, "branch": branch, "previous_head": previous_head, "head": head_oid(&repo)?, "branch_moved": moved, "refs_before": before, "refs_after": after, "visibility_setup": visibility_sync.as_str()}),
        lines,
    )
}

fn sync_pull_visibility(
    repo: &GitRepo,
    api: ApiSession<'_>,
    target: &ScopeRemote,
) -> anyhow::Result<WorktreeRepoConfigSync> {
    if worktree_scope_repo_config_presence(&repo.root)? == WorktreeRepoConfigPresence::Complete {
        load_worktree_scope_repo_config(&repo.root)?;
        load_worktree_scope_repo_config_base_hash(&repo.root)?;
        return Ok(WorktreeRepoConfigSync::Unchanged);
    }
    let summary = get_repo(api, &target.owner, &target.repo)?;
    let config = match summary.access.actor {
        RepositoryActor::Public => default_scope_repo_config(),
        RepositoryActor::Member | RepositoryActor::Owner => {
            get_repo_config(api, &target.owner, &target.repo)?.config
        }
    };
    sync_missing_worktree_scope_repo_config(&repo.root, &config)
}

fn tracked_branch(repo: &GitRepo, remote: &str, branch: &str) -> anyhow::Result<Option<String>> {
    if branch_config_value(repo, branch, "remote")?.as_deref() != Some(remote) {
        return Ok(None);
    }
    Ok(branch_config_value(repo, branch, "merge")?
        .and_then(|reference| reference.strip_prefix("refs/heads/").map(str::to_owned)))
}

fn remote_refs(repo: &GitRepo, remote: &str) -> anyhow::Result<BTreeMap<String, String>> {
    let prefix = format!("refs/remotes/{remote}");
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args([
            "for-each-ref",
            "--format=%(refname:strip=3) %(objectname)",
            &prefix,
        ])
        .output()
        .context("inspect Scope remote refs")?;
    if !output.status.success() {
        bail!("inspect Scope remote refs failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(name, _)| *name != "HEAD")
        .map(|(name, oid)| (name.to_string(), oid.to_string()))
        .collect())
}

fn ref_change_lines(
    remote: &str,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut changed = false;
    for (name, oid) in after {
        match before.get(name) {
            None => {
                changed = true;
                let kind = if name == DEFAULT_SCOPE_BRANCH {
                    "branch"
                } else {
                    "request"
                };
                lines.push(format!("  [new {kind}] {name} -> {remote}/{name}"));
            }
            Some(previous) if previous != oid => {
                changed = true;
                lines.push(format!(
                    "  [updated] {name} {}..{}",
                    short_oid(previous),
                    short_oid(oid)
                ));
            }
            _ => {}
        }
    }
    for name in before.keys().filter(|name| !after.contains_key(*name)) {
        changed = true;
        lines.push(format!("  [removed] {remote}/{name}"));
    }
    if !changed {
        lines.push("No remote refs changed.".to_owned());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;

    #[test]
    fn current_branch_must_track_the_selected_remote_before_merge() {
        let dir = TempDir::git_repo("pull-upstream", "main");
        dir.run_git(["config", "user.name", "Scope Test"]);
        dir.run_git(["config", "user.email", "scope@example.invalid"]);
        fs::write(dir.path().join("README.md"), "test\n").unwrap();
        dir.run_git(["add", "README.md"]);
        dir.run_git(["commit", "--quiet", "-m", "initial"]);
        dir.run_git([
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ]);
        dir.run_git(["update-ref", "refs/remotes/origin/main", "HEAD"]);
        dir.run_git(["config", "branch.main.remote", "origin"]);
        dir.run_git(["config", "branch.main.merge", "refs/heads/main"]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };

        assert_eq!(
            tracked_branch(&repo, "origin", "main").unwrap().as_deref(),
            Some("main")
        );
        assert_eq!(tracked_branch(&repo, "scope", "main").unwrap(), None);
        dir.run_git(["branch", "-m", "local-alias"]);
        assert_eq!(
            tracked_branch(&repo, "origin", "local-alias")
                .unwrap()
                .as_deref(),
            Some("main")
        );
    }

    #[test]
    fn linked_origin_tracking_stays_intact_while_visibility_is_initialized() {
        let main = TempDir::git_repo("pull-linked-main", "main");
        fs::write(main.path().join("README.md"), "initial\n").unwrap();
        main.run_git(["add", "README.md"]);
        main.run_git([
            "-c",
            "user.email=scope@example.test",
            "-c",
            "user.name=Scope Test",
            "commit",
            "--quiet",
            "-m",
            "initial",
        ]);
        let linked = main.path().join("linked");
        main.run_git([
            "worktree",
            "add",
            "-b",
            "contribution",
            linked.to_str().unwrap(),
        ]);
        main.run_git([
            "remote",
            "add",
            "origin",
            "https://github.example/owner/repo",
        ]);
        main.run_git(["config", "branch.contribution.remote", "origin"]);
        main.run_git(["config", "branch.contribution.merge", "refs/heads/main"]);
        let repo = GitRepo { root: linked };
        let before = head_oid(&repo).unwrap();
        let config = default_scope_repo_config();

        assert_eq!(
            tracked_branch(&repo, "scope", "contribution").unwrap(),
            None
        );
        assert_eq!(
            sync_missing_worktree_scope_repo_config(&repo.root, &config).unwrap(),
            WorktreeRepoConfigSync::Created
        );
        assert_eq!(head_oid(&repo).unwrap(), before);
        assert_eq!(
            branch_config_value(&repo, "contribution", "remote")
                .unwrap()
                .as_deref(),
            Some("origin")
        );
    }
}
