use super::{MergeRequestCommand, RequestMergeFailure};
use crate::{error::ApiError, state::AppState};
use scope_domain::{
    repository::access::{RepositoryAccess, RepositoryActor},
    requests::{
        MergeGateAuthority, MergeGateInput, Request, RequestViewer, evaluate_merge_gate,
        request_policy,
    },
};

pub(super) struct CheckedMerge {
    pub(super) context: scope_domain::repository::access::RepositoryAccessContext,
    pub(super) access: RepositoryAccess,
    pub(super) request: Request,
    is_invitee: bool,
}

pub(super) async fn authorize_merge(
    state: &AppState,
    command: &MergeRequestCommand,
) -> Result<CheckedMerge, RequestMergeFailure> {
    let context = crate::repo_access::find_read_access(
        state,
        &command.owner,
        &command.repo_name,
        Some(&command.actor_user_id),
    )
    .await?;
    let access = context.access.clone();
    let request = state
        .metadata
        .requests()
        .request_by_id(&command.request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    let is_invitee = state
        .metadata
        .requests()
        .request_is_invitee(&request.id, &command.actor_user_id)
        .await?;
    let viewer = RequestViewer::new(access.clone(), Some(&command.actor_user_id), is_invitee);
    let policy = request_policy(&request, viewer.clone(), &context.views);
    if request.repo_id != context.record.id || !policy.exact_visible {
        return Err(ApiError::not_found("request not found").into());
    }
    if !policy.permissions.can_merge {
        if matches!(access.actor, RepositoryActor::Public) {
            return Err(ApiError::forbidden("repo maintainer required").into());
        }
        return Err(ApiError::conflict("request cannot be merged").into());
    }
    if command
        .expected_head_oid
        .as_ref()
        .is_some_and(|expected| *expected != request.head_oid)
    {
        return Err(
            ApiError::conflict("request has a new revision; review it before merging").into(),
        );
    }
    Ok(CheckedMerge {
        context,
        access,
        request,
        is_invitee,
    })
}

pub(super) async fn ensure_checks_cleared(
    state: &AppState,
    command: &MergeRequestCommand,
    checked: &CheckedMerge,
) -> Result<(), RequestMergeFailure> {
    let CheckedMerge {
        context,
        access,
        request,
        is_invitee,
    } = checked;
    let viewer = RequestViewer::new(access.clone(), Some(&command.actor_user_id), *is_invitee);
    let checks =
        crate::use_cases::request_checks::checks_outcome(state, &context.record, request).await?;
    let decision = evaluate_merge_gate(MergeGateInput::RequestMerge {
        repository_lifecycle: context.record.lifecycle_state,
        subject: request.into(),
        viewer: &viewer,
        views: &context.views,
        checks,
        reviews: state
            .metadata
            .requests()
            .request_reviews_outcome(request, &context.views)
            .await?,
    });
    if decision.authority() != Some(MergeGateAuthority::RequestMerge) {
        return Err(
            ApiError::conflict(decision.reason().unwrap_or("request CI has not cleared")).into(),
        );
    }
    Ok(())
}
