use crate::{error::ApiError, repo_events::RepoChangeReason, state::AppState};
use scope_domain::{
    repository::access::RepositoryAccessContext, requests::RequestTimelineMutation,
};
use scope_postgres::db::EditRequestIdentityCommand;

pub(crate) async fn edit_request_identity(
    state: &AppState,
    repo: &RepositoryAccessContext,
    command: EditRequestIdentityCommand,
) -> Result<RequestTimelineMutation, ApiError> {
    let mutation = state
        .metadata
        .requests()
        .edit_request_identity(command)
        .await?;
    state
        .publish_request_summary_refresh(
            &repo.incarnation(),
            RepoChangeReason::RequestIdentityEdited,
        )
        .await;
    Ok(mutation)
}
