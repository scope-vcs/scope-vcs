use super::{
    actor_request, current_main_oid_for_context, current_main_oid_for_view, read_policy,
    request_viewer, summary::request_response_for_viewer, visible_request,
};
use crate::{
    auth::scope::require_scope_user, error::ApiError, persistence::unix_now, state::AppState,
    use_cases::request_work,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{AttachRequestChangeRequest, RequestMutationResponse};
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{
        AttachRequestChangeInput, RequestReference, RequestTimelineMutation, RequestWorkInput,
        ensure_named_requests_within_limit,
    },
};

pub(crate) async fn start_request_work(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let (user, repo, request) =
        actor_request(&state, &headers, &owner, &repo_name, &request_id).await?;
    let mutation = request_work::start_work(
        &state,
        &repo,
        RequestWorkInput {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            event_id: crate::persistence_ids::generate_prefixed_id("event_request_work_started")?,
            now_unix: unix_now()?,
        },
    )
    .await?;
    work_response(&state, &repo, &user.id, mutation).await
}

pub(crate) async fn release_request_work(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let (user, repo, request) =
        actor_request(&state, &headers, &owner, &repo_name, &request_id).await?;
    let mutation = request_work::release_work(
        &state,
        &repo,
        RequestWorkInput {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            event_id: crate::persistence_ids::generate_prefixed_id("event_request_work_released")?,
            now_unix: unix_now()?,
        },
    )
    .await?;
    work_response(&state, &repo, &user.id, mutation).await
}

pub(crate) async fn attach_request_change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, reference)): Path<(String, String, String)>,
    Json(input): Json<AttachRequestChangeRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = read_policy(&state, &owner, &repo_name, Some(&user.id)).await?;
    let request_id = referenced_request_id(&state, &repo.context.record.id, &reference).await?;
    let request = visible_request(&state, &repo.context, Some(&user.id), &request_id)
        .await?
        .0;
    let main_oid = current_main_oid_for_view(&state, &repo, request.view.clone())
        .await?
        .ok_or_else(|| ApiError::conflict("repo has no main branch to base a request on"))?;
    let mutation = request_work::attach_change(
        &state,
        &repo.context,
        AttachRequestChangeInput {
            request_id,
            actor_user_id: user.id.clone(),
            name: input.name,
            main_oid,
            event_id: crate::persistence_ids::generate_prefixed_id(
                "event_request_change_attached",
            )?,
            now_unix: unix_now()?,
        },
    )
    .await?;
    work_response(&state, &repo.context, &user.id, mutation).await
}

pub(super) async fn referenced_request_id(
    state: &AppState,
    repo_id: &str,
    reference: &str,
) -> Result<String, ApiError> {
    match RequestReference::parse(reference) {
        RequestReference::Id(request_id) => Ok(request_id),
        RequestReference::GitHubIssue(number) => Ok(state
            .metadata
            .requests()
            .request_by_github_issue_number(repo_id, number)
            .await?
            .map_or_else(|| reference.to_string(), |request| request.id)),
    }
}

pub(crate) async fn referenced_request_ids(
    state: &AppState,
    repo_id: &str,
    references: &[String],
) -> Result<Vec<String>, ApiError> {
    ensure_named_requests_within_limit(references.len())?;
    let mut request_ids = Vec::with_capacity(references.len());
    for reference in references {
        request_ids.push(referenced_request_id(state, repo_id, reference).await?);
    }
    Ok(request_ids)
}

async fn work_response(
    state: &AppState,
    repo: &RepositoryAccessContext,
    viewer_user_id: &str,
    mutation: RequestTimelineMutation,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let current_main_oid = current_main_oid_for_context(state, repo).await?;
    let viewer = request_viewer(
        state,
        &mutation.request.id,
        repo.access.clone(),
        Some(viewer_user_id),
    )
    .await?;
    let request = request_response_for_viewer(
        state,
        mutation.request,
        viewer,
        &repo.views,
        current_main_oid,
        repo.record.lifecycle_state,
    )
    .await?;
    Ok(Json(RequestMutationResponse { request }))
}
