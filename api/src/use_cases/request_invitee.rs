use crate::{error::ApiError, repo_events::RepoChangeReason, state::AppState};
use scope_domain::repository::access::RepositoryAccessContext;
use scope_postgres::db::{
    AddRequestInviteeCommand, RemoveRequestInviteeCommand, RequestInviteeRead,
};

pub(crate) async fn add_request_invitee(
    state: &AppState,
    repo: &RepositoryAccessContext,
    command: AddRequestInviteeCommand,
) -> Result<RequestInviteeRead, ApiError> {
    let invitee = state
        .metadata
        .requests()
        .add_request_invitee(command)
        .await?;
    state
        .publish_request_summary_refresh(&repo.incarnation(), RepoChangeReason::RequestInviteeAdded)
        .await;
    Ok(invitee)
}

pub(crate) async fn remove_request_invitee(
    state: &AppState,
    repo: &RepositoryAccessContext,
    command: RemoveRequestInviteeCommand,
) -> Result<RequestInviteeRead, ApiError> {
    let invitee = state
        .metadata
        .requests()
        .remove_request_invitee(command)
        .await?;
    state
        .publish_request_summary_refresh(
            &repo.incarnation(),
            RepoChangeReason::RequestInviteeRemoved,
        )
        .await;
    Ok(invitee)
}
