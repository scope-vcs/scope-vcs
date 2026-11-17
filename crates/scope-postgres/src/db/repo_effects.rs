use super::cleanup_queue::queue::{
    queue_pending_repo_storage_cleanup_row, queue_pending_source_blob_deletion_rows,
};
use sea_orm::ConnectionTrait;
use {crate::error::PostgresError, scope_domain::repo_actions::RepoEffect};

pub(super) async fn save_repo_effects<C>(
    conn: &C,
    effects: &[RepoEffect],
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    for effect in effects {
        match effect {
            RepoEffect::DeleteRepoStorage(cleanup) => {
                queue_pending_repo_storage_cleanup_row(conn, cleanup.clone(), now_unix).await?;
            }
            RepoEffect::DeleteSourceBlobs(blobs) => {
                queue_pending_source_blob_deletion_rows(conn, blobs.clone(), now_unix).await?;
            }
        }
    }

    Ok(())
}
