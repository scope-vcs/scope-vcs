use crate::{error::ApiError, persistence::unix_now, state::AppState};
use scope_domain::repository::repo_id;

pub(crate) async fn drain_pending_repo_storage_deletions(state: &AppState) -> Result<(), ApiError> {
    let metadata = state.metadata.clone();
    let cleanup_store = metadata.cleanup();
    let repository_store = metadata.repositories();
    let now_unix = unix_now()?;
    let state = state.clone();
    let batch = cleanup_store.repo_storage_cleanup_batch(now_unix).await?;
    let mut first_failure = None;
    let mut retained = Vec::new();
    for cleanup in &batch.pending {
        let cleanup_repo_id = repo_id(&cleanup.owner_handle, &cleanup.repo_name);
        let repository_store = repository_store.clone();
        let state = state.clone();
        let (live_repo, delete_result) = repository_store
            .with_repo_storage_lock(&cleanup_repo_id, || async {
                if repository_store.repository_exists(&cleanup_repo_id).await? {
                    return Ok::<_, ApiError>((true, Ok(())));
                }
                Ok::<_, ApiError>((
                    false,
                    crate::git::storage::delete_repo_storage(&state, cleanup),
                ))
            })
            .await?;
        if live_repo {
            retained.push(cleanup.clone());
            continue;
        }
        if let Err(error) = delete_result {
            tracing::warn!(?error, owner = %cleanup.owner_handle, repo = %cleanup.repo_name, "failed to clean deleted repo filesystem storage");
            first_failure.get_or_insert_with(|| {
                format!(
                    "failed to clean deleted repo storage {}/{}: {}",
                    cleanup.owner_handle,
                    cleanup.repo_name,
                    error.into_operator_diagnostic()
                )
            });
            retained.push(cleanup.clone());
        }
    }
    cleanup_store
        .finish_repo_storage_cleanup(batch, &retained, now_unix)
        .await?;
    first_failure.map_or(Ok(()), |failure| {
        Err(ApiError::infrastructure_unavailable(failure))
    })
}

pub(crate) async fn best_effort_drain_pending_repo_storage_deletions(state: &AppState) {
    if let Err(error) = drain_pending_repo_storage_deletions(state).await {
        tracing::warn!(?error, "failed to drain pending repo storage deletions");
    }
}
