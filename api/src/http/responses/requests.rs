use super::git_oid_response;
use crate::error::ApiError;
use scope_api_contract::{
    RequestActorSummaryResponse, RequestEventResponse, RequestInviteeResponse,
    RequestListItemResponse, RequestMergeabilityResponse, RequestPermissionsResponse,
    RequestSummaryResponse,
};
use scope_domain::{
    requests::{
        Request, RequestChecksOutcome, RequestEvent, RequestMergeSubject, RequestViewer,
        request_list_mergeability, request_mergeability, request_policy,
    },
    views::Views,
};
use scope_postgres::db::RequestListRow;

pub(crate) fn request_summary_response(
    request: Request,
    invitees: Vec<RequestInviteeResponse>,
    permissions: RequestPermissionsResponse,
    mergeability: RequestMergeabilityResponse,
) -> Result<RequestSummaryResponse, crate::error::ApiError> {
    let state = request.state();
    Ok(RequestSummaryResponse {
        id: request.id,
        name: request.name,
        title: request.title,
        description_markdown: request.description_markdown,
        author_user_id: request.author_user_id,
        author_role: request.author_role.into(),
        view: request.view.into(),
        base: match request.base {
            scope_domain::requests::RequestBase::Main { review_base_oid } => {
                scope_api_contract::RequestBase::Main {
                    review_base_oid: super::git_oid_response(review_base_oid)?,
                }
            }
        },
        base_version: request.base_version,
        main_ancestor_oid: super::git_oid_response(request.main_ancestor_oid)?,
        head_oid: super::git_oid_response(request.head_oid)?,
        state: state.into(),
        activity_version: request.activity_version,
        submitted_at_unix: request.submitted_at_unix,
        closed_at_unix: request.closed_at_unix,
        closed_by_user_id: request.closed_by_user_id,
        merged_at_unix: request.merged_at_unix,
        merged_by_user_id: request.merged_by_user_id,
        merged_head_oid: request
            .merged_head_oid
            .map(super::git_oid_response)
            .transpose()?,
        merged_main_oid: request
            .merged_main_oid
            .map(super::git_oid_response)
            .transpose()?,
        created_at_unix: request.created_at_unix,
        updated_at_unix: request.updated_at_unix,
        invitees,
        permissions,
        mergeability,
    })
}

pub(crate) fn request_list_item_response(
    request: RequestListRow,
    viewer: &RequestViewer<'_>,
    views: &Views,
    current_main_oid: Option<String>,
    checks: RequestChecksOutcome,
    repository_lifecycle: scope_domain::repository::RepoLifecycleState,
) -> Result<RequestListItemResponse, crate::error::ApiError> {
    let decision = request_list_mergeability(
        RequestMergeSubject {
            author_user_id: request.author_user_id.as_deref(),
            view: &request.view,
            state: request.state,
            has_git_snapshot: request.has_git_snapshot,
        },
        viewer,
        views,
        checks,
        repository_lifecycle,
    );
    let request_head_oid = super::git_oid_response(request.head_oid)?;
    Ok(RequestListItemResponse {
        id: request.id,
        name: request.name,
        title: request.title,
        author_user_id: request.author_user_id,
        author_role: request.author_role.into(),
        view: request.view.into(),
        head_oid: request_head_oid.clone(),
        state: request.state.into(),
        submitted_at_unix: request.submitted_at_unix,
        updated_at_unix: request.updated_at_unix,
        mergeability: RequestMergeabilityResponse {
            status: decision.status.into(),
            current_main_oid: current_main_oid.map(super::git_oid_response).transpose()?,
            request_head_oid,
            reason: decision.reason.map(str::to_string),
        },
    })
}

pub(crate) fn request_event_response(
    event: RequestEvent,
    actor: Option<RequestActorSummaryResponse>,
) -> RequestEventResponse {
    RequestEventResponse {
        id: event.id,
        position: event.position,
        actor,
        kind: event.kind.into(),
        payload: event.payload.into(),
        created_at_unix: event.created_at_unix,
    }
}

pub(crate) fn request_response(
    request: Request,
    viewer: RequestViewer<'_>,
    views: &Views,
    current_main_oid: Option<String>,
    invitees: Vec<RequestInviteeResponse>,
    checks: scope_domain::requests::RequestChecksOutcome,
    repository_lifecycle: scope_domain::repository::RepoLifecycleState,
) -> Result<RequestSummaryResponse, ApiError> {
    let policy = request_policy(&request, viewer.clone(), views);
    let can_view_activity = policy.activity_stream_visible;
    let decision = policy.permissions;
    let permissions = RequestPermissionsResponse {
        can_view_activity,
        can_open_discussion: decision.can_open_discussion,
        can_reply_to_discussion: decision.can_reply_to_discussion,
        can_wait_after_reply: decision.can_wait_after_reply,
        can_edit_identity: decision.can_edit_identity,
        can_pull_branch: decision.can_pull_branch,
        can_push_branch: decision.can_push_branch,
        can_submit: decision.can_submit,
        can_manage_invitees: decision.can_manage_invitees,
        can_leave_request: decision.can_leave_request,
        can_close: decision.can_close,
        can_merge: decision.can_merge,
    };
    let decision = request_mergeability(&request, &viewer, views, checks, repository_lifecycle);
    let mergeability = RequestMergeabilityResponse {
        status: decision.status.into(),
        current_main_oid: current_main_oid.map(git_oid_response).transpose()?,
        request_head_oid: git_oid_response(request.head_oid.clone())?,
        reason: decision.reason.map(str::to_string),
    };
    request_summary_response(request, invitees, permissions, mergeability)
}
