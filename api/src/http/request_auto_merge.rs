use super::{
    requests::{repo_metadata_and_access, visible_request},
    responses::{git_oid_response, request_actor_summary_response},
};
use crate::{
    auth::scope::require_scope_user, error::ApiError, state::AppState,
    use_cases::request_auto_merge,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{
    AuthorizeRequestAutoMergeRequest, CancelRequestAutoMergeRequest,
    RequestAutoMergeIntentResponse, RequestAutoMergeResponse,
};
use scope_domain::requests::{
    RequestViewer, request_auto_merge_can_cancel, request_auto_merge_can_enable, request_policy,
};

pub(crate) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestAutoMergeResponse>, ApiError> {
    let snapshot =
        super::request_state::load(&state, &headers, &owner, &repo_name, &request_id).await?;
    snapshot_response(&snapshot, super::request_state::viewer(&snapshot)).map(Json)
}

pub(crate) async fn authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<AuthorizeRequestAutoMergeRequest>,
) -> Result<Json<RequestAutoMergeResponse>, ApiError> {
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
    request_auto_merge::authorize(
        &state,
        &request.id,
        &user.id,
        input.expected_revision_id,
        input.expected_head_oid.as_str().to_string(),
    )
    .await?;
    let snapshot =
        super::request_state::load(&state, &headers, &owner, &repo_name, &request_id).await?;
    snapshot_response(&snapshot, super::request_state::viewer(&snapshot)).map(Json)
}

pub(crate) async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<CancelRequestAutoMergeRequest>,
) -> Result<Json<RequestAutoMergeResponse>, ApiError> {
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
    request_auto_merge::cancel(&state, &request.id, &user.id, input.expected_intent_id).await?;
    let snapshot =
        super::request_state::load(&state, &headers, &owner, &repo_name, &request_id).await?;
    snapshot_response(&snapshot, super::request_state::viewer(&snapshot)).map(Json)
}

pub(crate) fn snapshot_response(
    snapshot: &scope_postgres::db::RequestStateSnapshot,
    viewer: RequestViewer<'_>,
) -> Result<RequestAutoMergeResponse, ApiError> {
    let readiness = scope_domain::requests::request_auto_merge_readiness(
        &snapshot.request.id,
        &snapshot.request.head_oid,
        snapshot.evaluation.as_ref(),
        &snapshot.results,
    );
    let views = &snapshot.repository.views;
    let users = &snapshot.users;
    let can_merge = request_policy(&snapshot.request, viewer.clone(), views)
        .permissions
        .can_merge;
    let can_enable = request_auto_merge_can_enable(
        &snapshot.request,
        snapshot.revision.as_ref(),
        snapshot.auto_merge.as_ref(),
        readiness,
        can_merge,
    );
    let can_cancel = request_auto_merge_can_cancel(
        &snapshot.request,
        snapshot.auto_merge.as_ref(),
        viewer.access.is_maintainer(),
    );
    let waiting_reason = snapshot
        .auto_merge
        .as_ref()
        .filter(|intent| intent.is_active())
        .and_then(|_| readiness.waiting_reason_message())
        .map(str::to_string);
    let intent = match snapshot.auto_merge.as_ref() {
        Some(intent) => Some(RequestAutoMergeIntentResponse {
            actor: request_actor_summary_response(&intent.actor_user_id, users)?,
            id: intent.id.clone(),
            revision_id: intent.revision_id.clone(),
            head_oid: git_oid_response(intent.head_oid.clone())?,
            status: intent.status.into(),
            reason: intent.reason.map(Into::into),
            created_at_unix: intent.created_at_unix,
            updated_at_unix: intent.updated_at_unix,
        }),
        None => None,
    };
    Ok(RequestAutoMergeResponse {
        request_id: snapshot.request.id.clone(),
        revision_id: snapshot
            .revision
            .as_ref()
            .map(|revision| revision.id.clone()),
        head_oid: git_oid_response(snapshot.request.head_oid.clone())?,
        intent,
        waiting_reason,
        can_enable,
        can_cancel,
    })
}
