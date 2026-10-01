use super::{
    GeneratedIdSource,
    cleanup_queue::queue::{
        queue_pending_repo_storage_cleanup_row, queue_pending_source_blob_deletion_rows,
    },
};
use sea_orm::ConnectionTrait;
use {
    crate::error::PostgresError,
    scope_domain::repo_actions::{RepoEffect, RepoEffects},
};

pub async fn save_repo_effects<C>(
    conn: &C,
    effects: &RepoEffects,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    if effects.is_empty() {
        return Ok(());
    }

    for effect in effects.iter() {
        match effect {
            RepoEffect::DeleteRepoStorage(cleanup) => {
                queue_pending_repo_storage_cleanup_row(
                    conn,
                    cleanup.clone(),
                    now_unix,
                    generated_ids,
                )
                .await?;
            }
            RepoEffect::DeleteSourceBlobs(blobs) => {
                queue_pending_source_blob_deletion_rows(
                    conn,
                    blobs.clone(),
                    now_unix,
                    generated_ids,
                )
                .await?;
            }
        }
    }

    Ok(())
}
