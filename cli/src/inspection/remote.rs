use super::{DiagnosticState, Report, main_push_target, record};
use crate::{
    api::{self, ApiSession},
    git_repo::GitSnapshot,
    git_transport::ScopeRemote,
};
use std::{thread, time::Duration};

pub(super) fn inspect_remote(
    report: &mut Report,
    checkout: Option<&GitSnapshot>,
    target: Option<&ScopeRemote>,
    main_push_remote: Option<&ScopeRemote>,
) {
    let endpoint = report.api_url.clone();
    let token = match crate::auth::read_stored_session_token(&endpoint) {
        Ok(Some(token)) => token,
        Ok(None) => {
            record(
                report,
                "authentication",
                DiagnosticState::Problem,
                "Not signed in".into(),
                Some("Run scope login, or scope login --exchange-file PATH for automation".into()),
            );
            return;
        }
        Err(error) => {
            record(
                report,
                "authentication",
                DiagnosticState::Problem,
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
            record(
                report,
                "remote",
                DiagnosticState::Problem,
                error.to_string(),
                None,
            );
            return;
        }
    };
    let session = ApiSession::new(&client, &endpoint, &token);
    let (account, summary) = thread::scope(|scope| {
        let summary = target
            .map(|target| scope.spawn(|| api::get_repo(session, &target.owner, &target.repo)));
        (api::validate_session_token(session), summary.map(joined))
    });
    match account {
        Ok(Some(user)) => {
            record(
                report,
                "authentication",
                DiagnosticState::Ok,
                format!("Signed in as @{}", user.handle),
                None,
            );
            report.account = Some(user);
        }
        Ok(None) => {
            record(
                report,
                "authentication",
                DiagnosticState::Problem,
                "Saved session expired or was revoked".into(),
                Some("Run scope login".into()),
            );
            return;
        }
        Err(error) => {
            record(
                report,
                "remote",
                DiagnosticState::Unavailable,
                error.to_string(),
                Some(
                    "Retry when Scope is reachable; local state remains available with --offline"
                        .into(),
                ),
            );
            return;
        }
    }
    let (Some(target), Some(summary)) = (target, summary) else {
        return;
    };
    let summary = match summary {
        Ok(summary) => summary,
        Err(error) => {
            record(
                report,
                "repository access",
                DiagnosticState::Problem,
                error.to_string(),
                None,
            );
            return;
        }
    };
    let reads_full_view = match crate::repository_views::reads_full_view(&summary) {
        Ok(reads_full_view) => reads_full_view,
        Err(error) => {
            record(
                report,
                "repository views",
                DiagnosticState::Problem,
                format!("{error:#}"),
                None,
            );
            false
        }
    };
    let request_context = checkout.map(|checkout| {
        crate::request::RequestContext::new(Some(checkout.clone()), target.clone(), summary.clone())
    });
    let (server_config, request, history) = thread::scope(|scope| {
        let server_config = reads_full_view
            .then(|| scope.spawn(|| api::get_repo_config(session, &target.owner, &target.repo)));
        let request = request_context.map(|context| {
            scope.spawn(move || crate::request::inspect_current_request(session, &context?))
        });
        let history = reads_full_view.then(|| {
            scope.spawn(|| {
                api::run_history(session, &target.owner, &target.repo, None, None, 20, None)
            })
        });
        (
            server_config.map(joined),
            request.map(joined),
            history.map(joined),
        )
    });
    match server_config {
        Some(Ok(server)) => {
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
                    record(report, "visibility_drift", DiagnosticState::Problem, "Server visibility changed since local setup/review".into(), Some("Inspect scope visibility show and resolve local edits before scope push --main".into()));
                }
            }
        }
        Some(Err(error)) => record(
            report,
            "server visibility",
            DiagnosticState::Unavailable,
            error.to_string(),
            None,
        ),
        None => {}
    }
    report.main_push_target = main_push_remote
        .filter(|push| {
            crate::repository_views::repository_views(&summary.views).is_ok_and(|views| {
                crate::push::pushes_main_through(
                    &summary.access,
                    summary.lifecycle_state,
                    &views,
                    &push.view,
                )
            })
        })
        .map(main_push_target);
    report.repository = Some(summary);
    match request {
        Some(Ok(request)) => report.request = request,
        Some(Err(error)) => record(
            report,
            "request",
            DiagnosticState::Unavailable,
            error.to_string(),
            None,
        ),
        None => {}
    }
    let Some(history) = history else {
        record(
            report,
            "runs",
            DiagnosticState::NotChecked,
            "Workflow runs are available to repository maintainers".into(),
            None,
        );
        return;
    };
    match history {
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
                DiagnosticState::Ok,
                format!(
                    "{} matching runs in the 20 most recent repository runs",
                    report.recent_matching_runs.len()
                ),
                None,
            );
        }
        Err(error) => record(
            report,
            "runs",
            DiagnosticState::Unavailable,
            error.to_string(),
            None,
        ),
    }
}

fn joined<T>(handle: thread::ScopedJoinHandle<'_, T>) -> T {
    handle
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}
