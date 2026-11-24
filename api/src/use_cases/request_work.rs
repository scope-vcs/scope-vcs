use crate::after_commit::AfterCommit;
use crate::{error::ApiError, repo_events::RepoChangeReason, state::AppState};
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{
        AttachRequestChangeFacts, AttachRequestChangeInput, RequestTimelineMutation,
        RequestWorkInput, attach_request_change, release_request_work, start_request_work,
    },
};

pub(crate) async fn start_work(
    state: &AppState,
    repo: &RepositoryAccessContext,
    input: RequestWorkInput,
) -> Result<RequestTimelineMutation, ApiError> {
    let tx = state
        .metadata
        .requests()
        .begin_request(&input.request_id, &input.actor_user_id)
        .await?;
    let event_id_exists = tx.event_id_exists(&input.event_id).await?;
    let mutation = start_request_work(tx.request().clone(), tx.actor(), event_id_exists, input)?;
    saved(state, repo, tx, mutation).await
}

pub(crate) async fn release_work(
    state: &AppState,
    repo: &RepositoryAccessContext,
    input: RequestWorkInput,
) -> Result<RequestTimelineMutation, ApiError> {
    let tx = state
        .metadata
        .requests()
        .begin_request(&input.request_id, &input.actor_user_id)
        .await?;
    let event_id_exists = tx.event_id_exists(&input.event_id).await?;
    let mutation = release_request_work(tx.request().clone(), tx.actor(), event_id_exists, input)?;
    saved(state, repo, tx, mutation).await
}

pub(crate) async fn attach_change(
    state: &AppState,
    repo: &RepositoryAccessContext,
    input: AttachRequestChangeInput,
) -> Result<RequestTimelineMutation, ApiError> {
    let tx = state
        .metadata
        .requests()
        .begin_request_change_attach(&input.request_id, &input.actor_user_id)
        .await?;
    let facts = AttachRequestChangeFacts {
        event_id_exists: tx.event_id_exists(&input.event_id).await?,
        another_request_has_name: tx.another_request_has_name(&input.name).await?,
    };
    let mutation = attach_request_change(tx.request().clone(), tx.actor(), facts, input)?;
    saved(state, repo, tx, mutation).await
}

async fn saved(
    state: &AppState,
    repo: &RepositoryAccessContext,
    tx: scope_postgres::db::RequestTransaction,
    mutation: RequestTimelineMutation,
) -> Result<RequestTimelineMutation, ApiError> {
    tx.save_work_change(&mutation).await?;
    tx.commit().await?;
    state.after_commit([AfterCommit::request_summary(
        &repo.incarnation(),
        RepoChangeReason::RequestWorkChanged,
    )]);
    Ok(mutation)
}
