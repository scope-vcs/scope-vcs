use crate::{error::ApiError, persistence::unix_now, state::AppState};
use scope_content_lifecycle::SourceBlobCleanupReport;
use scope_domain::{content::SourceBlob, repository::repo_id};

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

pub(crate) async fn persist_pending_source_blob_deletions(
    state: &AppState,
    blobs: &[SourceBlob],
) -> Result<(), ApiError> {
    if blobs.is_empty() {
        return Ok(());
    }
    let blobs = blobs.to_vec();
    Ok(state
        .metadata
        .cleanup()
        .queue_pending_source_blob_deletions(blobs, unix_now()?)
        .await?)
}

pub(crate) async fn best_effort_cleanup_rollback_source_blobs(
    state: &AppState,
    blobs: &[SourceBlob],
) {
    if blobs.is_empty() {
        return;
    }
    if let Err(queue_error) = persist_pending_source_blob_deletions(state, blobs).await {
        tracing::warn!(?queue_error, "failed to queue rollback source blob cleanup");
    }
}

pub(crate) async fn drain_pending_source_blob_deletions(
    state: &AppState,
) -> Result<SourceBlobCleanupReport, ApiError> {
    Ok(scope_content_lifecycle::drain_source_blob_cleanup(
        &state.metadata,
        state.object_store.as_ref(),
        unix_now()?,
    )
    .await?)
}
