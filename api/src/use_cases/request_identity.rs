use crate::after_commit::AfterCommit;
use crate::{error::ApiError, repo_events::RepoChangeReason, state::AppState};
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{EditRequestIdentityInput, RequestTimelineMutation, edit_request_identity as edit},
};
use scope_postgres::{db::RequestStore, error::PostgresError};

pub(crate) async fn edit_request_identity(
    state: &AppState,
    repo: &RepositoryAccessContext,
    input: EditRequestIdentityInput,
) -> Result<RequestTimelineMutation, ApiError> {
    let mutation = crate::use_cases::request_identity::persist_identity_edit(
        &state.metadata.requests(),
        input,
    )
    .await?;
    state.after_commit([AfterCommit::request_summary(
        &repo.incarnation(),
        RepoChangeReason::RequestIdentityEdited,
    )]);
    Ok(mutation)
}

pub(crate) async fn persist_identity_edit(
    store: &RequestStore,
    input: EditRequestIdentityInput,
) -> Result<RequestTimelineMutation, PostgresError> {
    let tx = store
        .begin_request(&input.request_id, &input.actor_user_id)
        .await?;
    let event_id_exists = tx.event_id_exists(&input.event_id).await?;
    let now_unix = input.now_unix;
    let new_description_markdown = input.description_markdown.clone();
    let mutation = edit(tx.request().clone(), tx.actor(), event_id_exists, input)?;
    tx.save_identity_edit(&mutation, new_description_markdown.as_deref(), now_unix)
        .await?;
    tx.commit().await?;
    Ok(mutation)
}

#[cfg(test)]
mod tests;
