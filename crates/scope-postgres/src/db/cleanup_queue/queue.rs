use crate::{
    db::{
        CleanupStore, GeneratedIdKind, entities, generated_ids::generate_id,
        integer_columns::u64_to_i64,
    },
    error::PostgresError,
};
use scope_domain::{content::SourceBlob, repo_actions::RepoStorageCleanup};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set, TransactionTrait,
    sea_query::OnConflict,
};

pub(crate) const SOURCE_BLOB_DELETE_GRACE_SECONDS: u64 = 600;

impl CleanupStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "queue_pending_source_blob_deletions"))]
    pub async fn queue_pending_source_blob_deletions(
        &self,
        blobs: Vec<SourceBlob>,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        if blobs.is_empty() {
            return Ok(());
        }

        let tx = self.db.begin().await?;
        queue_pending_source_blob_deletion_rows(&tx, blobs, now_unix).await?;
        tx.commit().await?;
        Ok(())
    }
}

pub(crate) async fn queue_pending_repo_storage_cleanup_row<C>(
    conn: &C,
    cleanup: RepoStorageCleanup,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let generation = generate_id(GeneratedIdKind::CleanupGeneration)?;
    entities::repo_storage_cleanup_job::Entity::insert(
        entities::repo_storage_cleanup_job::Model::from_domain(&cleanup, generation, now_unix)?
            .into_active_model(),
    )
    .on_conflict(
        OnConflict::column(entities::repo_storage_cleanup_job::Column::RepoId)
            .update_columns([
                entities::repo_storage_cleanup_job::Column::Generation,
                entities::repo_storage_cleanup_job::Column::OwnerHandle,
                entities::repo_storage_cleanup_job::Column::RepoName,
                entities::repo_storage_cleanup_job::Column::IncarnationId,
                entities::repo_storage_cleanup_job::Column::Attempts,
                entities::repo_storage_cleanup_job::Column::NextRunAtUnix,
                entities::repo_storage_cleanup_job::Column::LastError,
                entities::repo_storage_cleanup_job::Column::CompletedAtUnix,
                entities::repo_storage_cleanup_job::Column::UpdatedAtUnix,
            ])
            .to_owned(),
    )
    .exec(conn)
    .await?;
    Ok(())
}

pub(crate) async fn queue_pending_source_blob_deletion_rows<C>(
    conn: &C,
    blobs: impl IntoIterator<Item = SourceBlob>,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let first_attempt = now_unix
        .checked_add(SOURCE_BLOB_DELETE_GRACE_SECONDS)
        .ok_or_else(|| {
            PostgresError::internal_message("source blob cleanup time exceeds u64 range")
        })?;
    let first_attempt = u64_to_i64(first_attempt, "source blob cleanup first attempt")?;
    for blob in blobs {
        let generation = generate_id(GeneratedIdKind::CleanupGeneration)?;
        let mut cleanup =
            entities::source_blob_cleanup_job::Model::from_domain(&blob, generation, now_unix)?
                .into_active_model();
        cleanup.next_run_at_unix = Set(first_attempt);
        entities::source_blob_cleanup_job::Entity::insert(cleanup)
            .on_conflict(
                OnConflict::column(entities::source_blob_cleanup_job::Column::ObjectKey)
                    .update_columns([
                        entities::source_blob_cleanup_job::Column::Generation,
                        entities::source_blob_cleanup_job::Column::Sha256,
                        entities::source_blob_cleanup_job::Column::GitOid,
                        entities::source_blob_cleanup_job::Column::SizeBytes,
                        entities::source_blob_cleanup_job::Column::Attempts,
                        entities::source_blob_cleanup_job::Column::NextRunAtUnix,
                        entities::source_blob_cleanup_job::Column::LastError,
                        entities::source_blob_cleanup_job::Column::CompletedAtUnix,
                        entities::source_blob_cleanup_job::Column::UpdatedAtUnix,
                    ])
                    .to_owned(),
            )
            .exec(conn)
            .await?;
    }
    Ok(())
}

#[cfg(any(test, feature = "seeding"))]
pub async fn save_pending_repo_storage_deletions<C>(
    conn: &C,
    pending_repo_storage_deletions: &[RepoStorageCleanup],
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    for cleanup in pending_repo_storage_deletions {
        queue_pending_repo_storage_cleanup_row(conn, cleanup.clone(), now_unix).await?;
    }
    Ok(())
}

pub async fn pending_repo_storage_cleanup_exists<C>(
    conn: &C,
    cleanup_repo_id: &str,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    let row = entities::repo_storage_cleanup_job::Entity::find_by_id(cleanup_repo_id.to_string())
        .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
        .one(conn)
        .await?;
    Ok(row.is_some())
}

#[cfg(any(test, feature = "seeding"))]
pub async fn save_pending_source_blob_deletions<C>(
    conn: &C,
    pending_source_blob_deletions: &[SourceBlob],
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    queue_pending_source_blob_deletion_rows(
        conn,
        pending_source_blob_deletions.iter().cloned(),
        now_unix,
    )
    .await
}
