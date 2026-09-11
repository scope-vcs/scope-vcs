use crate::persistence_ids::generate_prefixed_id;
use crate::use_cases::request_access::{request_policy_for_viewer, visible_request};
use crate::{
    auth::scope::{optional_scope_user, principal_for_scope_user, require_scope_user},
    error::ApiError,
    http::responses::*,
    persistence::unix_now,
    product_analytics::ProductEvent,
    repo_access::{ensure_repo_read, find_repo},
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::request_merge::{self, MergeRequestCommand, MergeRequestResult},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_api_contract::{
    AddRequestInviteeRequest, EditRequestIdentityRequest, LeaveRequestResponse,
    RemoveRequestInviteeRequest, RequestCloseResponse, RequestDetailResponse,
    RequestInviteeMutationResponse, RequestListResponse, RequestMergeabilityResponse,
    RequestMutationResponse, RequestPermissionsResponse, RequestSummaryResponse,
    StartRequestRequest, SubmitRequestRequest,
};
use scope_domain::{
    projection::{ProjectionViewKey, project_graph},
    repository::Repository,
    repository::access::{RepositoryAccess, RepositoryAccessContext, RepositoryActor},
    requests::{
        CloseRequestMutation, REQUEST_LIST_DEFAULT_PAGE_SIZE, REQUEST_LIST_MAX_PAGE_SIZE, Request,
        RequestAudience, RequestViewer, StartRequestInput, request_actor_role,
        request_mergeability, request_policy,
    },
};
use scope_postgres::db::{EditRequestIdentityCommand, SubmitRequestCommand};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct RequestListQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

pub(crate) async fn list_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<RequestListQuery>,
) -> Result<Json<RequestListResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let after_id = query
        .cursor
        .as_deref()
        .map(parse_request_list_cursor)
        .transpose()?;
    let limit = query
        .limit
        .unwrap_or(REQUEST_LIST_DEFAULT_PAGE_SIZE)
        .clamp(1, REQUEST_LIST_MAX_PAGE_SIZE);
    let mut requests = state
        .metadata
        .requests()
        .request_list_page(scope_postgres::db::RequestListPageQuery {
            repo_id: &repo.record.id,
            viewer_user_id: viewer_user_id.as_deref(),
            access,
            after_id: after_id.as_deref(),
            limit: (limit + 1) as u64,
        })
        .await?;
    let has_more = requests.len() > limit;
    requests.truncate(limit);
    let next_cursor = if has_more {
        requests
            .last()
            .map(|request| encode_request_list_cursor(&request.id))
    } else {
        None
    };
    let current_main_oid = if requests.is_empty() {
        None
    } else {
        current_main_oid_for_context(&state, &repo).await?
    };
    let requests = requests
        .into_iter()
        .map(|request| request_list_item_response(request, access, current_main_oid.clone()))
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(Json(RequestListResponse {
        requests,
        next_cursor,
    }))
}

fn parse_request_list_cursor(value: &str) -> Result<String, ApiError> {
    value
        .strip_prefix("v1:")
        .filter(|id| !id.is_empty() && !id.contains(':'))
        .map(str::to_string)
        .ok_or_else(|| ApiError::bad_request("invalid request list cursor"))
}

fn encode_request_list_cursor(last_id: &str) -> String {
    format!("v1:{last_id}")
}

pub(crate) async fn get_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestDetailResponse>, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let request = visible_request(
        &state,
        &repo.record.id,
        access,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    let current_main_oid = current_main_oid_for_context(&state, &repo).await?;
    let request = request_response_for_viewer(
        &state,
        request,
        access,
        current_main_oid,
        viewer_user_id.as_deref(),
    )
    .await?;

    Ok(Json(RequestDetailResponse { request }))
}

pub(crate) async fn submit_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(_input): Json<SubmitRequestRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let request =
        visible_request(&state, &repo.record.id, access, Some(&user.id), &request_id).await?;
    let analytics_event =
        ProductEvent::request_submitted(&user.id, request.audience, request_actor_role(access));
    let current_main_oid = current_main_oid_for_context(&state, &repo).await?;
    let mutation = state
        .metadata
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            event_id: generate_prefixed_id("event_request_submitted_")?,
            now_unix: unix_now()?,
        })
        .await?;
    state.product_analytics.capture(analytics_event);
    lifecycle_response(
        &state,
        &repo,
        access,
        &user.id,
        mutation.request,
        current_main_oid,
        RepoChangeReason::RequestSubmitted,
    )
    .await
}

pub(crate) async fn merge_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let result = request_merge::merge_request(
        &state,
        MergeRequestCommand {
            owner,
            repo_name,
            request_id,
            actor_user_id: user.id,
        },
    )
    .await?;
    merge_response(&state, result).await
}

async fn merge_response(
    state: &AppState,
    result: MergeRequestResult,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let current_main_oid = committed_main_oid_for_access(&result.repo, result.access)?;
    let request = request_response_for_viewer(
        state,
        result.request,
        result.access,
        current_main_oid,
        Some(&result.actor_user_id),
    )
    .await?;
    Ok(Json(RequestMutationResponse { request }))
}

async fn lifecycle_response(
    state: &AppState,
    repo: &RepositoryAccessContext,
    access: RepositoryAccess,
    viewer_user_id: &str,
    request: Request,
    current_main_oid: Option<String>,
    refresh_reason: RepoChangeReason,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let request = request_response_for_viewer(
        state,
        request,
        access,
        current_main_oid,
        Some(viewer_user_id),
    )
    .await?;
    state
        .publish_request_summary_refresh(&repo.incarnation(), refresh_reason)
        .await;
    Ok(Json(RequestMutationResponse { request }))
}

fn committed_main_oid_for_access(
    repo: &Repository,
    access: RepositoryAccess,
) -> Result<Option<String>, ApiError> {
    if access.actor != RepositoryActor::Public
        && access.can_read_private_files
        && let Some(head) = repo.git_head.as_ref()
    {
        return Ok(Some(head.head_oid.clone()));
    }
    let projection = project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        ProjectionViewKey::from_access(access),
    );
    scope_git::projection_head_oid(&projection).map_err(ApiError::internal)
}

pub(crate) async fn close_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestCloseResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let request =
        visible_request(&state, &repo.record.id, access, Some(&user.id), &request_id).await?;
    if !request_policy(&request, RequestViewer::new(access, Some(&user.id), false))
        .permissions
        .can_close
    {
        return Err(ApiError::forbidden("request close access required"));
    }
    let current_main_oid = current_main_oid_for_context(&state, &repo).await?;
    let mutation =
        crate::use_cases::request_close::close_request(&state, &repo, &request, &user.id).await?;
    match mutation {
        CloseRequestMutation::DeletedDraft { .. } => Ok(Json(RequestCloseResponse {
            deleted: true,
            request: None,
        })),
        CloseRequestMutation::Closed { request, .. } => {
            let request = request_response_for_viewer(
                &state,
                request,
                access,
                current_main_oid,
                Some(&user.id),
            )
            .await?;
            Ok(Json(RequestCloseResponse {
                deleted: false,
                request: Some(request),
            }))
        }
    }
}

pub(crate) async fn start_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<StartRequestRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = find_repo(&state, &owner, &repo_name).await?;
    let principal = principal_for_scope_user(&repo, Some(&user));
    ensure_repo_read(&state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    let audience: RequestAudience = input.audience.into();
    if access.actor == RepositoryActor::Public && audience != RequestAudience::Public {
        return Err(ApiError::forbidden(
            "public contributors can only create public requests",
        ));
    }
    let base_main_oid = current_main_oid_for_audience(&state, &repo, audience)
        .await?
        .ok_or_else(|| ApiError::conflict("repo has no main branch to base a request on"))?;
    let request_id = generate_prefixed_id("req_")?;
    let now_unix = unix_now()?;
    let mutation = state
        .metadata
        .requests()
        .start_request(StartRequestInput {
            id: request_id.clone(),
            repo_id: repo.record.id.clone(),
            name: input.name,
            author_user_id: user.id.clone(),
            title: input.title,
            author_role: request_actor_role(access),
            audience,
            base_main_oid,
            event_id: generate_prefixed_id("event_request_started_")?,
            now_unix,
        })
        .await?;
    state
        .product_analytics
        .capture(ProductEvent::request_started(
            &user.id,
            audience,
            request_actor_role(access),
        ));
    let current_main_oid = committed_main_oid_for_access(&repo, access)?;
    let request = request_response_for_viewer(
        &state,
        mutation.request,
        access,
        current_main_oid,
        Some(&user.id),
    )
    .await?;
    state
        .publish_request_summary_refresh(&repo.incarnation(), RepoChangeReason::RequestStarted)
        .await;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn edit_request_identity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<EditRequestIdentityRequest>,
) -> Result<Json<RequestMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    let request =
        visible_request(&state, &repo.record.id, access, Some(&user.id), &request_id).await?;
    let current_main_oid = current_main_oid_for_context(&state, &repo).await?;
    let mutation = state
        .metadata
        .requests()
        .edit_request_identity(EditRequestIdentityCommand {
            request_id: request.id,
            actor_user_id: user.id.clone(),
            event_id: generate_prefixed_id("event_request_identity_edited_")?,
            title: input.title,
            description_markdown: input.description_markdown,
            expected_description_markdown: input.expected_description_markdown,
            now_unix: unix_now()?,
        })
        .await?;
    let request = request_response_for_viewer(
        &state,
        mutation.request,
        access,
        current_main_oid,
        Some(&user.id),
    )
    .await?;
    state
        .publish_request_summary_refresh(
            &repo.incarnation(),
            RepoChangeReason::RequestIdentityEdited,
        )
        .await;
    Ok(Json(RequestMutationResponse { request }))
}

pub(crate) async fn add_request_invitee(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<AddRequestInviteeRequest>,
) -> Result<Json<RequestInviteeMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    visible_request(&state, &repo.record.id, access, Some(&user.id), &request_id).await?;
    let current_main_oid = current_main_oid_for_context(&state, &repo).await?;
    let invitee = state
        .metadata
        .requests()
        .add_request_invitee(scope_postgres::db::AddRequestInviteeCommand {
            request_id: request_id.clone(),
            actor_user_id: user.id.clone(),
            target_handle: input.handle,
            now_unix: unix_now()?,
        })
        .await?;
    invitee_mutation_response(
        &state,
        &repo,
        &user.id,
        request_id,
        invitee,
        current_main_oid,
        RepoChangeReason::RequestInviteeAdded,
    )
    .await
}

pub(crate) async fn remove_request_invitee(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<RemoveRequestInviteeRequest>,
) -> Result<Json<RequestInviteeMutationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    visible_request(&state, &repo.record.id, access, Some(&user.id), &request_id).await?;
    let current_main_oid = current_main_oid_for_context(&state, &repo).await?;
    let invitee = state
        .metadata
        .requests()
        .remove_request_invitee(scope_postgres::db::RemoveRequestInviteeCommand {
            request_id: request_id.clone(),
            actor_user_id: user.id.clone(),
            target_handle: input.handle,
        })
        .await?;
    invitee_mutation_response(
        &state,
        &repo,
        &user.id,
        request_id,
        invitee,
        current_main_oid,
        RepoChangeReason::RequestInviteeRemoved,
    )
    .await
}

pub(crate) async fn leave_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<LeaveRequestResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let (repo, access, _) = repo_metadata_and_access(&state, &headers, &owner, &repo_name).await?;
    visible_request(&state, &repo.record.id, access, Some(&user.id), &request_id).await?;
    let invitee = state
        .metadata
        .requests()
        .leave_request(scope_postgres::db::LeaveRequestCommand {
            request_id: request_id.clone(),
            actor_user_id: user.id.clone(),
        })
        .await?;
    let invitee = request_invitee_response(invitee);
    state
        .publish_request_summary_refresh(&repo.incarnation(), RepoChangeReason::RequestInviteeLeft)
        .await;
    Ok(Json(LeaveRequestResponse { invitee }))
}

async fn invitee_mutation_response(
    state: &AppState,
    repo: &RepositoryAccessContext,
    viewer_user_id: &str,
    request_id: String,
    invitee: scope_postgres::db::RequestInviteeRead,
    current_main_oid: Option<String>,
    refresh_reason: RepoChangeReason,
) -> Result<Json<RequestInviteeMutationResponse>, ApiError> {
    let request = state
        .metadata
        .requests()
        .request_by_id(&request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    let request = request_response_for_viewer(
        state,
        request,
        repo.access,
        current_main_oid,
        Some(viewer_user_id),
    )
    .await?;
    let invitee = request_invitee_response(invitee);
    state
        .publish_request_summary_refresh(&repo.incarnation(), refresh_reason)
        .await;
    Ok(Json(RequestInviteeMutationResponse { request, invitee }))
}

pub(crate) async fn repo_and_access(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<(Repository, RepositoryAccess, Option<String>), ApiError> {
    let repo = find_repo(state, owner, repo_name).await?;
    let user = optional_scope_user(state, headers).await?;
    let principal = user
        .as_ref()
        .map(|user| principal_for_scope_user(&repo, Some(user)))
        .unwrap_or_else(scope_domain::policy::Principal::public);
    ensure_repo_read(state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    Ok((repo, access, user.map(|user| user.id)))
}

async fn request_response_for_viewer(
    state: &AppState,
    request: Request,
    access: RepositoryAccess,
    current_main_oid: Option<String>,
    viewer_user_id: Option<&str>,
) -> Result<RequestSummaryResponse, ApiError> {
    let policy = request_policy_for_viewer(state, &request, access, viewer_user_id).await?;
    let invitees = if request.audience == RequestAudience::Public && policy.exact_visible {
        state
            .metadata
            .requests()
            .request_invitees(&request.id)
            .await?
            .into_iter()
            .map(request_invitee_response)
            .collect()
    } else {
        Vec::new()
    };
    let can_view_activity = policy.activity_stream_visible;
    let decision = policy.permissions;
    let permissions = RequestPermissionsResponse {
        can_view_activity,
        can_open_discussion: decision.can_open_discussion,
        can_reply_to_discussion: decision.can_reply_to_discussion,
        can_edit_identity: decision.can_edit_identity,
        can_pull_branch: decision.can_pull_branch,
        can_push_branch: decision.can_push_branch,
        can_submit: decision.can_submit,
        can_manage_invitees: decision.can_manage_invitees,
        can_leave_request: decision.can_leave_request,
        can_close: decision.can_close,
        can_merge: decision.can_merge,
    };
    let decision = request_mergeability(&request, access);
    let mergeability = RequestMergeabilityResponse {
        status: decision.status.into(),
        current_main_oid: current_main_oid.map(git_oid_response).transpose()?,
        request_head_oid: git_oid_response(request.head_oid.clone())?,
        reason: decision.reason.map(str::to_string),
    };
    request_summary_response(request, invitees, permissions, mergeability)
}

pub(crate) async fn current_main_oid_for_audience(
    state: &AppState,
    repo: &Repository,
    audience: RequestAudience,
) -> Result<Option<String>, ApiError> {
    if audience == RequestAudience::Private
        && let Some(head) = repo.git_head.as_ref()
    {
        return Ok(Some(head.head_oid.clone()));
    }
    let view_key = match audience {
        RequestAudience::Private => ProjectionViewKey::Private,
        RequestAudience::Public => ProjectionViewKey::Public,
    };
    state
        .metadata
        .repositories()
        .live_projection_head_oid(repo, view_key)
        .await
        .map_err(Into::into)
}

pub(crate) async fn repo_metadata_and_access(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
) -> Result<(RepositoryAccessContext, RepositoryAccess, Option<String>), ApiError> {
    let user = optional_scope_user(state, headers).await?;
    let repo = crate::repo_access::find_read_access(
        state,
        owner,
        repo_name,
        user.as_ref().map(|user| user.id.as_str()),
    )
    .await?;
    let access = repo.access;
    Ok((repo, access, user.map(|user| user.id)))
}

pub(crate) async fn current_main_oid_for_context(
    state: &AppState,
    repo: &RepositoryAccessContext,
) -> Result<Option<String>, ApiError> {
    Ok(state
        .metadata
        .repositories()
        .repository_main_oid(repo)
        .await?)
}
