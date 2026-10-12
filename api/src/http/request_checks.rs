use super::{
    requests::{repo_metadata_and_access, visible_request},
    responses::git_oid_response,
};
use crate::{
    auth::scope::require_scope_user, error::ApiError, persistence::unix_now, state::AppState,
    use_cases::request_checks,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{
    ApproveRequestChecksRequest, RequestCheckResponse, RequestCheckRunResponse,
    RequestChecksResponse, RequestGitHubPushResponse, RequestGitHubPushState,
    RequestMergeabilityResponse,
};
use scope_domain::{
    github_connection::GitHubRepositoryVisibility,
    github_workflow_jobs::github_run_visible,
    repository::access::RepositoryAccess,
    requests::{
        GitHubBranch, GitHubPushStatus, RequestCheck, RequestCheckResults, RequestCheckReviewer,
        RequestViewer, request_checks_message, request_mergeability,
    },
};
use scope_postgres::db::{ApproveRequestChecksCommand, GitHubWorkflowRunReference};
use std::collections::HashMap;

pub(crate) async fn get_request_checks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestChecksResponse>, ApiError> {
    let snapshot =
        super::request_state::load(&state, &headers, &owner, &repo_name, &request_id).await?;
    snapshot_response(
        &state,
        &snapshot,
        super::request_state::viewer(&snapshot),
        unix_now()?,
    )
    .map(Json)
}

pub(crate) async fn approve_request_checks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<ApproveRequestChecksRequest>,
) -> Result<Json<RequestChecksResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let (request, _) = visible_request(
        &state,
        &repo.record.id,
        &repo.views,
        access.clone(),
        Some(&user.id),
        &request_id,
    )
    .await?;
    request_checks::checks_view(&state, &repo.record, &request).await?;
    let mutation = state
        .metadata
        .requests()
        .approve_request_checks(ApproveRequestChecksCommand {
            request_id: request.id.clone(),
            actor_user_id: user.id.clone(),
            reviewed_head_oid: input.expected_head_oid.as_str().to_string(),
            now_unix: unix_now()?,
        })
        .await?;
    request_checks::publish_request_checks_change(&state, &repo.incarnation(), &mutation).await;
    let snapshot =
        super::request_state::load(&state, &headers, &owner, &repo_name, &request_id).await?;
    snapshot_response(
        &state,
        &snapshot,
        super::request_state::viewer(&snapshot),
        unix_now()?,
    )
    .map(Json)
}

pub(crate) fn snapshot_response(
    state: &AppState,
    snapshot: &scope_postgres::db::RequestStateSnapshot,
    viewer: RequestViewer<'_>,
    now_unix: u64,
) -> Result<RequestChecksResponse, ApiError> {
    let request = &snapshot.request;
    let views = &snapshot.repository.views;
    let current_main_oid = snapshot.current_main_oid.clone();
    let access = viewer.access.clone();
    let evaluation = &snapshot.evaluation;
    let results = &snapshot.results;
    let outcome = scope_domain::requests::request_checks_outcome(
        &request.id,
        &request.head_oid,
        evaluation.as_ref(),
        results,
    );
    let decision = request_mergeability(request, &viewer, views, outcome);
    let mergeability = RequestMergeabilityResponse {
        status: decision.status.into(),
        current_main_oid: current_main_oid.map(git_oid_response).transpose()?,
        request_head_oid: git_oid_response(request.head_oid.clone())?,
        reason: decision.reason.map(str::to_string),
    };
    let github_connection = &snapshot.github_connection;
    let private_request_on_public_github = views.anyone() != Some(&request.view)
        && github_connection.as_ref().is_some_and(|connection| {
            connection.is_connected()
                && connection.visibility != GitHubRepositoryVisibility::Private
        });
    let Some(evaluation) = evaluation else {
        return Ok(RequestChecksResponse {
            request_id: request.id.clone(),
            head_oid: git_oid_response(request.head_oid.clone())?,
            state: None,
            message: None,
            checks: Vec::new(),
            can_approve: false,
            github_push: None,
            changes_github_workflows: false,
            private_request_on_public_github,
            mergeability,
        });
    };
    let can_approve =
        evaluation.awaits_approval() && RequestCheckReviewer::may_review(&access, views);
    let latest_push = &snapshot.github_push;
    let github_push = GitHubPushStatus::for_evaluation(evaluation, latest_push.as_ref())
        .map(|status| github_push_response(&request.id, status, access.clone()));
    let changes_github_workflows = can_approve
        && evaluation.asks_github()
        && evaluation.changes_github_workflows.unwrap_or(true);
    let check_runs = if github_connection.as_ref().is_some_and(|connection| {
        state.github.is_some() && github_run_visible(&access, views, connection.visibility, true)
    }) {
        &snapshot.github_runs
    } else {
        &HashMap::new()
    };
    Ok(RequestChecksResponse {
        request_id: request.id.clone(),
        head_oid: git_oid_response(request.head_oid.clone())?,
        state: Some(evaluation.state.into()),
        message: request_checks_message(evaluation, results, latest_push.as_ref(), now_unix),
        checks: evaluation
            .checks
            .iter()
            .map(|check| check_response(check, &evaluation.tested_oid, results, check_runs))
            .collect(),
        can_approve,
        github_push,
        changes_github_workflows,
        private_request_on_public_github,
        mergeability,
    })
}

fn github_push_response(
    request_id: &str,
    status: GitHubPushStatus,
    access: RepositoryAccess,
) -> RequestGitHubPushResponse {
    let (state, error) = match status {
        GitHubPushStatus::AwaitingApproval => (RequestGitHubPushState::AwaitingApproval, None),
        GitHubPushStatus::Sending { last_error } => (RequestGitHubPushState::Sending, last_error),
        GitHubPushStatus::Sent => (RequestGitHubPushState::Sent, None),
        GitHubPushStatus::Failed { error } => (RequestGitHubPushState::Failed, Some(error)),
    };
    RequestGitHubPushResponse {
        state,
        branch: GitHubBranch::Request(request_id.to_string()).name(),
        error: error.filter(|_| access.is_maintainer()),
    }
}

fn check_response(
    check: &RequestCheck,
    tested_oid: &str,
    results: &RequestCheckResults,
    check_runs: &HashMap<u64, GitHubWorkflowRunReference>,
) -> RequestCheckResponse {
    match check {
        RequestCheck::Native(check) => RequestCheckResponse::Native {
            workflow_path: check.workflow_path.clone(),
            workflow_name: check.workflow_name.clone(),
            run_id: check.run_id.clone(),
            run_state: check.run_id.as_deref().and_then(|run_id| {
                results
                    .native_runs
                    .iter()
                    .find(|(id, _)| id == run_id)
                    .map(|(_, state)| (*state).into())
            }),
        },
        RequestCheck::GitHub { name } => {
            let run = results.github.latest(tested_oid, name);
            RequestCheckResponse::GitHub {
                name: name.clone(),
                status: run.map(|run| run.status.into()),
                conclusion: run.and_then(|run| run.conclusion.map(Into::into)),
                details_url: run.and_then(|run| run.details_url.clone()),
                run: run.and_then(|run| {
                    let workflow_run = check_runs.get(&run.check_suite_id?)?;
                    Some(RequestCheckRunResponse {
                        run_id: workflow_run.github_run_id.to_string(),
                        workflow_name: workflow_run.workflow_name.clone(),
                        job_id: run.github_check_run_id.to_string(),
                    })
                }),
            }
        }
    }
}
