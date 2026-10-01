use super::{
    requests::{current_main_oid_for_context, repo_metadata_and_access, visible_request},
    responses::git_oid_response,
};
use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    persistence::unix_now,
    state::AppState,
    use_cases::request_checks::{self, RequestChecksView},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{
    RequestCheckResponse, RequestChecksResponse, RequestGitHubPushResponse, RequestGitHubPushState,
    RequestMergeabilityResponse,
};
use scope_domain::{
    repository::{RepoRecord, access::RepositoryAccess},
    requests::{
        GitHubPushStatus, Request, RequestCheck, RequestCheckResults, github_request_branch,
        request_checks_message, request_mergeability,
    },
};
use scope_postgres::db::ApproveRequestChecksCommand;

pub(crate) async fn get_request_checks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestChecksResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let (request, _) = visible_request(
        &state,
        &repo.record.id,
        access,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    let current_main_oid = current_main_oid_for_context(&state, &repo).await?;
    checks_response(&state, &repo.record, &request, access, current_main_oid)
        .await
        .map(Json)
}

pub(crate) async fn approve_request_checks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestChecksResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let (request, _) =
        visible_request(&state, &repo.record.id, access, Some(&user.id), &request_id).await?;
    // Approving is a look too: a head nobody evaluated is evaluated before approval.
    request_checks::checks_view(&state, &repo.record, &request).await?;
    let mutation = state
        .metadata
        .requests()
        .approve_request_checks(ApproveRequestChecksCommand {
            request_id: request.id.clone(),
            actor_user_id: user.id.clone(),
            now_unix: unix_now()?,
        })
        .await?;
    request_checks::publish_request_checks_change(&state, &repo.incarnation(), &mutation).await;
    let current_main_oid = current_main_oid_for_context(&state, &repo).await?;
    checks_response(&state, &repo.record, &request, access, current_main_oid)
        .await
        .map(Json)
}

async fn checks_response(
    state: &AppState,
    repo: &RepoRecord,
    request: &Request,
    access: RepositoryAccess,
    current_main_oid: Option<String>,
) -> Result<RequestChecksResponse, ApiError> {
    let RequestChecksView {
        evaluation,
        results,
        outcome,
    } = request_checks::readable_checks_view(state, repo, request).await?;
    let decision = request_mergeability(request, access, outcome);
    let mergeability = RequestMergeabilityResponse {
        status: decision.status.into(),
        current_main_oid: current_main_oid.map(git_oid_response).transpose()?,
        request_head_oid: git_oid_response(request.head_oid.clone())?,
        reason: decision.reason.map(str::to_string),
    };
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
            mergeability,
        });
    };
    let can_approve = evaluation.awaits_approval() && access.is_maintainer();
    let latest_push = state
        .metadata
        .requests()
        .latest_github_push(&request.id)
        .await?;
    let github_push = GitHubPushStatus::for_evaluation(&evaluation, latest_push.as_ref())
        .map(|status| github_push_response(&request.id, status, access));
    let changes_github_workflows = can_approve
        && evaluation.asks_github()
        && request_checks::changes_github_workflow_files(state, repo, request).await;
    Ok(RequestChecksResponse {
        request_id: request.id.clone(),
        head_oid: git_oid_response(request.head_oid.clone())?,
        state: Some(evaluation.state.into()),
        message: request_checks_message(&evaluation, &results),
        checks: evaluation
            .checks
            .iter()
            .map(|check| check_response(check, &evaluation.tested_oid, &results))
            .collect(),
        can_approve,
        github_push,
        changes_github_workflows,
        mergeability,
    })
}

/// What GitHub answered can name private repositories and paths, so only
/// maintainers read it.
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
        branch: github_request_branch(request_id),
        error: error.filter(|_| access.is_maintainer()),
    }
}

fn check_response(
    check: &RequestCheck,
    tested_oid: &str,
    results: &RequestCheckResults,
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
            }
        }
    }
}
