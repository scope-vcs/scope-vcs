//! Read-only checkout status and setup diagnostics.
use crate::{
    api, context, execution,
    git_repo::{self, GitRepo},
    repo_config,
};
use anyhow::Context;
use serde::Serialize;
use std::{path::PathBuf, process::Command, time::Duration};

#[derive(Serialize)]
struct LocalState {
    root: PathBuf,
    branch: Option<String>,
    head_oid: Option<String>,
    dirty: bool,
    upstream: Option<String>,
    unpushed_commits: Option<u64>,
    visibility: Option<VisibilityState>,
}

#[derive(Serialize)]
struct VisibilityState {
    path: PathBuf,
    local_hash: String,
    base_hash: Option<String>,
    local_edits: Option<bool>,
    server_hash: Option<String>,
    server_changed: Option<bool>,
}

#[derive(Serialize)]
struct Diagnostic {
    name: &'static str,
    state: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<String>,
}

#[derive(Serialize)]
struct Report {
    api_url: String,
    offline: bool,
    healthy: bool,
    account: Option<api::UserResponse>,
    local: Option<LocalState>,
    repository: Option<api::RepoSummaryResponse>,
    target: Option<String>,
    main_push_target: Option<String>,
    request: Option<api::RequestSummaryResponse>,
    recent_matching_runs: Vec<api::RepositoryRunSummaryResponse>,
    diagnostics: Vec<Diagnostic>,
    next_actions: Vec<String>,
}

pub fn status(remote: Option<&str>, offline: bool) -> anyhow::Result<()> {
    let report = inspect(remote, offline);
    let lines = status_lines(&report);
    execution::emit("status", &report, lines)
}

pub fn doctor(remote: Option<&str>, offline: bool) -> anyhow::Result<()> {
    let report = inspect(remote, offline);
    let mut lines = vec![format!("Scope API: {}", report.api_url)];
    for check in &report.diagnostics {
        lines.push(format!(
            "{} · {}: {}",
            check.state, check.name, check.message
        ));
        if let Some(recovery) = &check.recovery {
            lines.push(format!("  Next: {recovery}"));
        }
    }
    lines.push(
        if report.healthy {
            "No setup problems found in the checks performed."
        } else {
            "Setup needs attention; follow the actions above."
        }
        .to_string(),
    );
    execution::emit("doctor", &report, lines)
}

fn inspect(remote: Option<&str>, offline: bool) -> Report {
    let endpoint = api::api_url();
    let mut report = Report {
        api_url: endpoint.clone(),
        offline,
        healthy: true,
        account: None,
        local: None,
        repository: None,
        target: None,
        main_push_target: None,
        request: None,
        recent_matching_runs: Vec::new(),
        diagnostics: Vec::new(),
        next_actions: Vec::new(),
    };
    let valid_endpoint = match context::validate_api_url(&endpoint) {
        Ok(()) => {
            record(&mut report, "endpoint", "ok", endpoint.clone(), None);
            true
        }
        Err(error) => {
            report.api_url = "invalid endpoint".into();
            record(
                &mut report,
                "endpoint",
                "problem",
                error.to_string(),
                Some("Set --api-url to a valid Scope HTTP(S) endpoint".into()),
            );
            false
        }
    };
    let git_available = match Command::new("git").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let supported = git_version_supported(&version);
            record(
                &mut report,
                "git",
                if supported { "ok" } else { "problem" },
                version,
                (!supported)
                    .then(|| "Install Git 2.38 or newer for request merge inspection".into()),
            );
            true
        }
        _ => {
            record(
                &mut report,
                "git",
                "problem",
                "Git is not available on PATH".into(),
                Some("Install Git 2.38 or newer".into()),
            );
            false
        }
    };
    let checkout = if git_available {
        match context::discover_optional() {
            Ok(repo) => repo,
            Err(error) => {
                record(&mut report, "checkout", "problem", error.to_string(), None);
                None
            }
        }
    } else {
        None
    };
    if let Some(repo) = &checkout {
        match local_state(repo) {
            Ok(local) => {
                record(
                    &mut report,
                    "checkout",
                    "ok",
                    local.root.display().to_string(),
                    None,
                );
                if local.head_oid.is_none() {
                    record(
                        &mut report,
                        "commit",
                        "problem",
                        "No committed HEAD".into(),
                        Some("Create a Git commit before publishing or running a workflow".into()),
                    );
                }
                report.local = Some(local);
            }
            Err(error) => record(&mut report, "checkout", "problem", error.to_string(), None),
        }
        match local_visibility(repo) {
            Ok(visibility) => {
                let message = if visibility.local_edits == Some(true) { "Local visibility changes have not been published" } else { "Local visibility config is valid" };
                record(&mut report, "visibility", "ok", message.into(), None);
                if let Some(local) = &mut report.local { local.visibility = Some(visibility); }
            }
            Err(error) => record(&mut report, "visibility", "problem", error.to_string(), Some("Inspect setup with scope visibility show; use scope init for a new repository or repair the retained repository setup".into())),
        }
        if let Some(head) = report
            .local
            .as_ref()
            .and_then(|local| local.head_oid.clone())
        {
            match crate::agent_context::ensure_repo_rules_ready_for_push(&repo.root, &head) {
                Ok(()) => record(&mut report, "rules", "ok", "Contribution rules and agent files are synchronized in the worktree and committed HEAD".into(), None),
                Err(error) => record(&mut report, "rules", "problem", error.to_string(), Some("Run scope rules sync, then commit the generated files".into())),
            }
        }
    } else {
        record(
            &mut report,
            "checkout",
            "info",
            "No local checkout; remote-only commands can use --repo owner/repo".into(),
            None,
        );
    }
    let target = if valid_endpoint {
        match context::resolve_repository(checkout.as_ref(), remote) {
            Ok(target) => {
                report.target = Some(format!("{}/{}", target.owner, target.repo));
                if !target.remote.is_empty() {
                    report.main_push_target = Some(format!("{}/main", target.remote));
                }
                record(
                    &mut report,
                    "repository",
                    "ok",
                    format!("{}/{}", target.owner, target.repo),
                    None,
                );
                if let Some(repo) = &checkout {
                    check_fetch_auth(&mut report, repo, &target);
                }
                Some(target)
            }
            Err(error) => {
                record(&mut report, "repository", "problem", error.to_string(), Some("Choose --remote NAME or --repo owner/repo; initialize a new Scope repository with scope init --name NAME".into()));
                None
            }
        }
    } else {
        None
    };
    if offline {
        record(
            &mut report,
            "remote",
            "not_checked",
            "Offline mode; account, server visibility, requests, and runs were not queried".into(),
            None,
        );
    } else if valid_endpoint {
        inspect_remote(&mut report, checkout.as_ref(), target.as_ref());
    }
    if let Some(request) = &report.request {
        report.next_actions.push(
            if request.permissions.can_push_branch {
                "Commit changes, then scope request push"
            } else {
                "Inspect scope request diff and scope request checks"
            }
            .into(),
        );
    } else if report
        .local
        .as_ref()
        .is_some_and(|local| local.head_oid.is_some())
        && report.target.is_some()
    {
        report.next_actions.push("Start a contribution with scope request start NAME, or publish deliberately with scope push --main".into());
    }
    report
}

fn inspect_remote(
    report: &mut Report,
    checkout: Option<&GitRepo>,
    target: Option<&crate::git_transport::ScopeRemote>,
) {
    let endpoint = report.api_url.clone();
    let token = match crate::auth::read_stored_session_token(&endpoint) {
        Ok(Some(token)) => token,
        Ok(None) => {
            record(
                report,
                "authentication",
                "problem",
                "Not signed in".into(),
                Some("Run scope login, or scope login --exchange-file PATH for automation".into()),
            );
            return;
        }
        Err(error) => {
            record(
                report,
                "authentication",
                "problem",
                error.to_string(),
                Some("Check that the credential store is available".into()),
            );
            return;
        }
    };
    let client = match api::http_client_builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            record(report, "remote", "problem", error.to_string(), None);
            return;
        }
    };
    match api::validate_session_token(&client, &endpoint, &token) {
        Ok(Some(user)) => {
            record(
                report,
                "authentication",
                "ok",
                format!("Signed in as @{}", user.handle),
                None,
            );
            report.account = Some(user);
        }
        Ok(None) => {
            record(
                report,
                "authentication",
                "problem",
                "Saved session expired or was revoked".into(),
                Some("Run scope login".into()),
            );
            return;
        }
        Err(error) => {
            record(
                report,
                "remote",
                "unavailable",
                error.to_string(),
                Some(
                    "Retry when Scope is reachable; local state remains available with --offline"
                        .into(),
                ),
            );
            return;
        }
    }
    let Some(target) = target else {
        return;
    };
    let summary = match api::get_repo(&client, &endpoint, &token, &target.owner, &target.repo) {
        Ok(summary) => summary,
        Err(error) => {
            record(
                report,
                "repository access",
                "problem",
                error.to_string(),
                None,
            );
            return;
        }
    };
    if summary.access.actor != api::RepositoryActor::Public {
        match api::get_repo_config(&client, &endpoint, &token, &target.owner, &target.repo) {
            Ok(server) => {
                if let Some(visibility) = report
                    .local
                    .as_mut()
                    .and_then(|local| local.visibility.as_mut())
                {
                    visibility.server_changed = visibility
                        .base_hash
                        .as_ref()
                        .map(|base| base != &server.config_hash);
                    visibility.server_hash = Some(server.config_hash);
                    if visibility.server_changed == Some(true) {
                        record(report, "visibility drift", "problem", "Server visibility changed since local setup/review".into(), Some("Inspect scope visibility show and resolve local edits before scope push --main".into()));
                    }
                }
            }
            Err(error) => record(
                report,
                "server visibility",
                "unavailable",
                error.to_string(),
                None,
            ),
        }
    }
    report.repository = Some(summary);
    if let Some(checkout) = checkout {
        let remote = (!target.remote.is_empty()).then_some(target.remote.as_str());
        match crate::request::inspect_current_request(checkout, &client, &endpoint, &token, remote)
        {
            Ok(request) => report.request = request,
            Err(error) => record(report, "request", "unavailable", error.to_string(), None),
        }
    }
    if report
        .repository
        .as_ref()
        .is_some_and(|repo| repo.access.actor == api::RepositoryActor::Public)
    {
        record(
            report,
            "runs",
            "not_checked",
            "Workflow runs are available to repository maintainers".into(),
            None,
        );
        return;
    }
    match api::run_history(
        &client,
        &endpoint,
        &token,
        &target.owner,
        &target.repo,
        None,
        20,
        None,
    ) {
        Ok(history) => {
            let head = report
                .request
                .as_ref()
                .map(|request| request.head_oid.as_str())
                .or_else(|| {
                    report
                        .local
                        .as_ref()
                        .and_then(|local| local.head_oid.as_deref())
                });
            report.recent_matching_runs = history
                .runs
                .into_iter()
                .filter(|run| head.is_none_or(|head| run.git_oid == head))
                .collect();
            record(
                report,
                "runs",
                "ok",
                format!(
                    "{} matching runs in the 20 most recent repository runs",
                    report.recent_matching_runs.len()
                ),
                None,
            );
        }
        Err(error) => record(report, "runs", "unavailable", error.to_string(), None),
    }
}

fn record(
    report: &mut Report,
    name: &'static str,
    state: &'static str,
    message: String,
    recovery: Option<String>,
) {
    if matches!(state, "problem" | "unavailable") {
        report.healthy = false;
    }
    report.diagnostics.push(Diagnostic {
        name,
        state,
        message,
        recovery,
    });
}

fn local_state(repo: &GitRepo) -> anyhow::Result<LocalState> {
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
    let unpushed_commits = upstream
        .as_ref()
        .and_then(|_| git_text(repo, &["rev-list", "--count", "@{upstream}..HEAD"]))
        .and_then(|count| count.parse().ok());
    Ok(LocalState {
        root: repo.root.clone(),
        branch: git_repo::current_branch(repo).ok(),
        head_oid: git_text(repo, &["rev-parse", "--verify", "HEAD"]),
        dirty: !output.stdout.is_empty(),
        upstream,
        unpushed_commits,
        visibility: None,
    })
}

fn local_visibility(repo: &GitRepo) -> anyhow::Result<VisibilityState> {
    let config = repo_config::load_worktree_scope_repo_config(&repo.root)?;
    let local_hash = scope_domain::repo_config::repo_config_fingerprint(&config)?;
    let base_hash = Some(repo_config::load_worktree_scope_repo_config_base_hash(
        &repo.root,
    )?);
    let local_edits = base_hash.as_ref().map(|base| base != &local_hash);
    Ok(VisibilityState {
        path: repo_config::repo_config_path(&repo.root)?,
        local_hash,
        base_hash,
        local_edits,
        server_hash: None,
        server_changed: None,
    })
}

fn git_text(repo: &GitRepo, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args(args)
        .output()
        .ok()?;
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

fn check_fetch_auth(
    report: &mut Report,
    repo: &GitRepo,
    target: &crate::git_transport::ScopeRemote,
) {
    if target.remote.is_empty() {
        return;
    }
    let helper = git_text(
        repo,
        &[
            "config",
            "--local",
            "--get-urlmatch",
            "credential.helper",
            &target.permissioned_url,
        ],
    );
    if helper.as_deref() == Some("!scope git-credential") {
        record(
            report,
            "Git authentication",
            "ok",
            "Repository credential helper configured".into(),
            None,
        );
    } else {
        record(
            report,
            "Git authentication",
            "problem",
            "Scope credential helper is missing for the permissioned remote".into(),
            Some("Use scope pull to configure permissioned fetch authentication".into()),
        );
    }
}

fn status_lines(report: &Report) -> Vec<String> {
    let mut lines = vec![format!("Scope API: {}", report.api_url)];
    if let Some(account) = &report.account {
        lines.push(format!("Account: @{}", account.handle));
    }
    if let Some(target) = &report.target {
        lines.push(format!("Repository: {target}"));
    }
    if let Some(local) = &report.local {
        lines.push(format!("Checkout: {}", local.root.display()));
        lines.push(format!(
            "Branch: {} · HEAD {} · {}",
            local.branch.as_deref().unwrap_or("detached"),
            local
                .head_oid
                .as_deref()
                .map(|oid| &oid[..oid.len().min(12)])
                .unwrap_or("none"),
            if local.dirty {
                "uncommitted changes"
            } else {
                "clean"
            }
        ));
        if let Some(count) = local.unpushed_commits {
            lines.push(format!("Unpushed commits: {count}"));
        }
        if let Some(visibility) = &local.visibility {
            lines.push(format!(
                "Visibility: {} · {}",
                visibility.path.display(),
                if visibility.local_edits == Some(true) {
                    "local edits"
                } else {
                    "no known local edits"
                }
            ));
        }
    }
    if let Some(target) = &report.main_push_target {
        lines.push(format!("scope push --main destination: {target}"));
    }
    if let Some(request) = &report.request {
        lines.push(format!(
            "Request: {} · {:?} · {}",
            request.name, request.state, request.id
        ));
    }
    for run in &report.recent_matching_runs {
        lines.push(format!(
            "Run: {} · {} · {:?}",
            run.id, run.workflow_name, run.state
        ));
    }
    for check in &report.diagnostics {
        if matches!(check.state, "problem" | "unavailable" | "not_checked") {
            lines.push(format!("{}: {}", check.name, check.message));
        }
    }
    lines.extend(
        report
            .next_actions
            .iter()
            .map(|action| format!("Next: {action}")),
    );
    lines
}

fn git_version_supported(version: &str) -> bool {
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
