use super::{MergeRequestCommand, RequestMergeFailure};
use crate::{error::ApiError, state::AppState};
use scope_domain::requests::{
    MergeFacts, MergeReadinessFacts, MergeRefusal, MergeStep, Request, RequestActor, decide_merge,
};

pub(super) struct CheckedMerge {
    pub(super) context: scope_domain::repository::access::RepositoryAccessContext,
    pub(super) request: Request,
    is_invitee: bool,
}

impl CheckedMerge {
    fn decide(
        &self,
        command: &MergeRequestCommand,
        step: MergeStep<'_>,
    ) -> Result<(), RequestMergeFailure> {
        match decide_merge(
            MergeFacts {
                request: &self.request,
                repository_id: &self.context.record.id,
                repository_lifecycle: self.context.record.lifecycle_state,
                actor: RequestActor {
                    access: &self.context.access,
                    is_invitee: self.is_invitee,
                    views: &self.context.views,
                },
                actor_user_id: &command.actor_user_id,
            },
            step,
        ) {
            Ok(_) => Ok(()),
            Err(MergeRefusal::Rejected(error)) => Err(ApiError::from(error).into()),
            Err(
                MergeRefusal::RepositoryMoved
                | MergeRefusal::ReadinessUnavailable
                | MergeRefusal::AutoMergeStopped { .. },
            ) => Err(ApiError::internal_message("merge gate refused like a commit").into()),
        }
    }
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
    let checked = CheckedMerge {
        context,
        request,
        is_invitee,
    };
    checked.decide(
        command,
        MergeStep::Authorize {
            expected_head_oid: command.expected_head_oid.as_deref(),
        },
    )?;
    Ok(checked)
}

pub(super) async fn ensure_checks_cleared(
    state: &AppState,
    command: &MergeRequestCommand,
    checked: &CheckedMerge,
) -> Result<(), RequestMergeFailure> {
    let CheckedMerge {
        context, request, ..
    } = checked;
    let checks = crate::use_cases::request_checks::checks_view(state, &context.record, request)
        .await?
        .outcome;
    let view_mains = state
        .metadata
        .requests()
        .request_view_mains(&request.repo_id)
        .await?;
    let reviews = state
        .metadata
        .requests()
        .request_reviews_outcome(request, &context.views)
        .await?;
    checked.decide(
        command,
        MergeStep::Clear(MergeReadinessFacts {
            view_mains: &view_mains,
            checks,
            reviews,
        }),
    )
}
