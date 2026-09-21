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
    RequestCheckEvaluationState, RequestCheckResponse, RequestChecksResponse,
    RequestMergeabilityResponse,
};
use scope_domain::{
    repository::{RepoRecord, access::RepositoryAccess},
    requests::{Request, request_mergeability},
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
        run_states,
        outcome,
    } = request_checks::readable_checks_view(state, repo, request).await?;
    let decision = request_mergeability(request, access, outcome);
    let mergeability = RequestMergeabilityResponse {
        status: decision.status.into(),
        current_main_oid: current_main_oid.map(git_oid_response).transpose()?,
        request_head_oid: git_oid_response(request.head_oid.clone())?,
        reason: decision.reason.map(str::to_string),
    };
    let (evaluation_state, message, checks) = match evaluation {
        Some(evaluation) => (
            Some(evaluation.state.into()),
            evaluation.message,
            evaluation
                .checks
                .into_iter()
                .map(|check| RequestCheckResponse {
                    workflow_path: check.workflow_path,
                    workflow_name: check.workflow_name,
                    run_state: check.run_id.as_deref().and_then(|run_id| {
                        run_states
                            .iter()
                            .find(|(id, _)| id == run_id)
                            .map(|(_, state)| (*state).into())
                    }),
                    run_id: check.run_id,
                })
                .collect(),
        ),
        None => (None, None, Vec::new()),
    };
    Ok(RequestChecksResponse {
        request_id: request.id.clone(),
        head_oid: git_oid_response(request.head_oid.clone())?,
        state: evaluation_state,
        message,
        checks,
        can_approve: evaluation_state == Some(RequestCheckEvaluationState::AwaitingApproval)
            && access.is_maintainer(),
        mergeability,
    })
}
