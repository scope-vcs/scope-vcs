//! HTTP authorization and response projection for request auto-merge.

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
use scope_domain::{
    repository::access::RepositoryAccess,
    requests::{Request, request_auto_merge_can_cancel, request_auto_merge_can_enable},
};

pub(crate) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestAutoMergeResponse>, ApiError> {
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
    response(&state, &request, access).await.map(Json)
}

pub(crate) async fn authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<AuthorizeRequestAutoMergeRequest>,
) -> Result<Json<RequestAutoMergeResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let (request, _) =
        visible_request(&state, &repo.record.id, access, Some(&user.id), &request_id).await?;
    request_auto_merge::authorize(
        &state,
        &request.id,
        &user.id,
        input.expected_revision_id,
        input.expected_head_oid.as_str().to_string(),
    )
    .await?;
    // The receipt says what is persisted now. Execution survives this HTTP request.
    response(&state, &request, access).await.map(Json)
}

pub(crate) async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<CancelRequestAutoMergeRequest>,
) -> Result<Json<RequestAutoMergeResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let (request, _) =
        visible_request(&state, &repo.record.id, access, Some(&user.id), &request_id).await?;
    request_auto_merge::cancel(&state, &request.id, &user.id, input.expected_intent_id).await?;
    response(&state, &request, access).await.map(Json)
}

async fn response(
    state: &AppState,
    request: &Request,
    access: RepositoryAccess,
) -> Result<RequestAutoMergeResponse, ApiError> {
    let view = request_auto_merge::view(state, &request.id).await?;
    let can_enable = request_auto_merge_can_enable(
        &view.request,
        view.revision.as_ref(),
        view.intent.as_ref(),
        access.is_maintainer(),
    );
    let can_cancel =
        request_auto_merge_can_cancel(&view.request, view.intent.as_ref(), access.is_maintainer());
    let waiting_reason = view
        .intent
        .as_ref()
        .filter(|intent| intent.is_active())
        .and_then(|_| view.readiness.waiting_reason_message())
        .map(str::to_string);
    let intent = match view.intent {
        Some(intent) => {
            let users = state
                .metadata
                .auth()
                .users_by_ids([intent.actor_user_id.clone()])
                .await?;
            Some(RequestAutoMergeIntentResponse {
                actor: request_actor_summary_response(&intent.actor_user_id, &users)?,
                id: intent.id,
                revision_id: intent.revision_id,
                head_oid: git_oid_response(intent.head_oid)?,
                status: intent.status.into(),
                reason: intent.reason.map(Into::into),
                created_at_unix: intent.created_at_unix,
                updated_at_unix: intent.updated_at_unix,
            })
        }
        None => None,
    };
    Ok(RequestAutoMergeResponse {
        request_id: view.request.id,
        revision_id: view.revision.map(|revision| revision.id),
        head_oid: git_oid_response(view.request.head_oid)?,
        intent,
        waiting_reason,
        can_enable,
        can_cancel,
    })
}
