use crate::{
    api, context, execution,
    git_repo::{GitRepo, GitSnapshot},
};
use serde::Serialize;
use std::{path::PathBuf, process::Command};

mod local;
mod remote;
mod visibility;
use local::{compare_scope_ref, git_text, local_state};
use remote::inspect_remote;
use visibility::inspect_visibility;

#[derive(Serialize)]
struct LocalState {
    root: PathBuf,
    branch: Option<String>,
    head_oid: Option<String>,
    dirty: bool,
    upstream: Option<String>,
    comparison_ref: Option<String>,
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

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum DiagnosticState {
    Ok,
    Info,
    Problem,
    Unavailable,
    NotChecked,
}

impl std::fmt::Display for DiagnosticState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ok => "ok",
            Self::Info => "info",
            Self::Problem => "problem",
            Self::Unavailable => "unavailable",
            Self::NotChecked => "not_checked",
        })
    }
}

#[derive(Serialize)]
struct Diagnostic {
    name: &'static str,
    state: DiagnosticState,
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
    recent_matching_runs: Vec<api::RepositoryRunListItemResponse>,
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
    let (endpoint, endpoint_error) = match api::api_url().and_then(|endpoint| {
        context::validate_api_url(&endpoint)?;
        Ok(endpoint)
    }) {
        Ok(endpoint) => (endpoint, None),
        Err(error) => ("invalid endpoint".to_string(), Some(error)),
    };
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
    let valid_endpoint = match endpoint_error {
        None => {
            record(
                &mut report,
                "endpoint",
                DiagnosticState::Ok,
                endpoint.clone(),
                None,
            );
            true
        }
        Some(error) => {
            record(
                &mut report,
                "endpoint",
                DiagnosticState::Problem,
                error.to_string(),
                Some("Set --api-url to a valid Scope HTTP(S) endpoint".into()),
            );
            false
        }
    };
    let git_available = match Command::new("git").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let supported = crate::git_version::supported(&version);
            record(
                &mut report,
                "git",
                if supported {
                    DiagnosticState::Ok
                } else {
                    DiagnosticState::Problem
                },
                version,
                (!supported).then(|| {
                    format!(
                        "Install Git {} or newer",
                        crate::git_version::minimum_version_text()
                    )
                }),
            );
            true
        }
        _ => {
            record(
                &mut report,
                "git",
                DiagnosticState::Problem,
                "Git is not available on PATH".into(),
                Some(format!(
                    "Install Git {} or newer",
                    crate::git_version::minimum_version_text()
                )),
            );
            false
        }
    };
    let checkout = if git_available {
        match context::discover_optional().and_then(|repo| {
            repo.map(|repo| Ok((GitSnapshot::read(&repo)?, repo)))
                .transpose()
        }) {
            Ok(checkout) => checkout,
            Err(error) => {
                record(
                    &mut report,
                    "checkout",
                    DiagnosticState::Problem,
                    error.to_string(),
                    None,
                );
                None
            }
        }
    } else {
        None
    };
    if let Some((snapshot, repo)) = &checkout {
        match local_state(repo, snapshot) {
            Ok(local) => {
                record(
                    &mut report,
                    "checkout",
                    DiagnosticState::Ok,
                    local.root.display().to_string(),
                    None,
                );
                if local.head_oid.is_none() {
                    record(
                        &mut report,
                        "commit",
                        DiagnosticState::Problem,
                        "No committed HEAD".into(),
                        Some("Create a Git commit before publishing or running a workflow".into()),
                    );
                }
                report.local = Some(local);
            }
            Err(error) => record(
                &mut report,
                "checkout",
                DiagnosticState::Problem,
                error.to_string(),
                None,
            ),
        }
        inspect_visibility(&mut report, repo);
    } else {
        record(
            &mut report,
            "checkout",
            DiagnosticState::Info,
            "No local checkout; remote-only commands can use --repo owner/repo".into(),
            None,
        );
    }
    let mut comparison_main_remote = None;
    let mut main_push_remote = None;
    let target = if valid_endpoint {
        match context::resolve_repository(
            checkout.as_ref().map(|(snapshot, _)| snapshot),
            &endpoint,
            remote,
        ) {
            Ok(target) => {
                report.target = Some(format!("{}/{}", target.owner, target.repo));
                record(
                    &mut report,
                    "repository",
                    DiagnosticState::Ok,
                    format!("{}/{}", target.owner, target.repo),
                    None,
                );
                if let Some((snapshot, repo)) = &checkout {
                    main_push_remote =
                        context::select_scope_remote(snapshot, &endpoint, remote, true).ok();
                    comparison_main_remote = Some(
                        main_push_remote
                            .as_ref()
                            .map_or_else(|| target.remote.clone(), |push| push.remote.clone()),
                    );
                    report.main_push_target = main_push_remote
                        .as_ref()
                        .filter(|push| push.view.is_private())
                        .map(main_push_target);
                    check_fetch_auth(&mut report, repo, &target);
                }
                Some(target)
            }
            Err(error) => {
                record(&mut report, "repository", DiagnosticState::Problem, error.to_string(), Some("Choose --remote NAME or --repo owner/repo; initialize a new Scope repository with scope init --name NAME".into()));
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
            DiagnosticState::NotChecked,
            "Offline mode; account, server visibility, requests, and runs were not queried".into(),
            None,
        );
    } else if valid_endpoint {
        inspect_remote(
            &mut report,
            checkout.as_ref().map(|(snapshot, _)| snapshot),
            target.as_ref(),
            main_push_remote.as_ref(),
        );
    }
    let mut request_comparison_unavailable = false;
    if let (Some((snapshot, repo)), Some(target), Some(main_remote), Some(local)) = (
        checkout.as_ref(),
        target.as_ref(),
        comparison_main_remote.as_deref(),
        report.local.as_mut(),
    ) && let Some(request_id) = compare_scope_ref(
        local,
        repo,
        snapshot,
        target,
        main_remote,
        report.request.as_ref(),
    ) {
        request_comparison_unavailable = true;
        let branch = local.branch.clone().unwrap_or_else(|| "BRANCH".into());
        let reattach = format!("scope request checkout --request {request_id} --branch {branch}");
        record(
            &mut report,
            "request comparison",
            DiagnosticState::Unavailable,
            format!("Request {request_id} has no saved comparison ref"),
            Some(format!("Re-attach with {reattach}")),
        );
        report
            .next_actions
            .push(format!("Re-attach with {reattach}"));
    }
    if let Some(request) = &report.request {
        report.next_actions.push(
            if request.permissions.can_push_branch {
                "Commit changes, then scope request push"
            } else {
                "Inspect scope request diff and scope request ci"
            }
            .into(),
        );
    } else if !request_comparison_unavailable
        && report
            .local
            .as_ref()
            .is_some_and(|local| local.head_oid.is_some())
        && report.target.is_some()
    {
        report.next_actions.push(if report.main_push_target.is_some() {
            "Start a contribution with scope request start NAME, or publish deliberately with scope push --main"
        } else {
            "Start a contribution with scope request start NAME"
        }.into());
    }
    report
}

fn main_push_target(remote: &crate::git_transport::ScopeRemote) -> String {
    format!("{}/main", remote.remote)
}

fn record(
    report: &mut Report,
    name: &'static str,
    state: DiagnosticState,
    message: String,
    recovery: Option<String>,
) {
    if matches!(
        state,
        DiagnosticState::Problem | DiagnosticState::Unavailable
    ) {
        report.healthy = false;
    }
    report.diagnostics.push(Diagnostic {
        name,
        state,
        message,
        recovery,
    });
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
            &target.url(),
        ],
    );
    if helper.as_deref() == Some("!scope git-credential") {
        record(
            report,
            "git_authentication",
            DiagnosticState::Ok,
            "Repository credential helper configured".into(),
            None,
        );
    } else {
        record(
            report,
            "git_authentication",
            DiagnosticState::Problem,
            "Scope credential helper is missing for the Scope remote".into(),
            Some("Use scope pull to configure fetch authentication".into()),
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
                .map(crate::display::short_oid)
                .unwrap_or("none"),
            if local.dirty {
                "uncommitted changes"
            } else {
                "clean"
            }
        ));
        if let Some(count) = local.unpushed_commits {
            lines.push(format!(
                "Unpushed commits: {count} relative to {}",
                local.comparison_ref.as_deref().unwrap_or("Scope")
            ));
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
        lines.push(format!(
            "scope push --main destination: {target}{}",
            if report.repository.is_none() {
                " (server access not checked)"
            } else {
                ""
            }
        ));
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
        if matches!(
            check.state,
            DiagnosticState::Problem | DiagnosticState::Unavailable | DiagnosticState::NotChecked
        ) {
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
