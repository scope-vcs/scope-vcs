use super::super::{RepositoryStore, entities};
use crate::{
    db::integer_columns::{u32_to_i32, u64_to_i64},
    error::PostgresError,
};
use scope_domain::repository::git::{GitSegmentRef, GitSegmentUpload};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Statement,
};

impl RepositoryStore {
    pub async fn begin_git_segment_upload(
        &self,
        repo_id: &str,
        segment_id: &str,
        object_key: &str,
        encoding_version: u32,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        require_text(repo_id, "Git segment repository id")?;
        require_text(segment_id, "Git segment id")?;
        require_text(object_key, "Git segment object key")?;
        if encoding_version == 0 {
            return Err(PostgresError::internal_message(
                "Git segment encoding version must be positive",
            ));
        }
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_git_segment_uploads (
                    segment_id, repo_id, object_key, state, sha256,
                    plaintext_bytes, encrypted_bytes, encoding_version,
                    created_at_unix, updated_at_unix
                 )
                 SELECT $1, $2, $3, 'uploading', NULL, NULL, NULL, $4, $5, $5
                 FROM scope_repositories WHERE id = $2",
                [
                    segment_id.into(),
                    repo_id.into(),
                    object_key.into(),
                    u32_to_i32(encoding_version, "Git segment encoding version")?.into(),
                    u64_to_i64(now_unix, "Git segment upload creation time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)
            .and_then(|result| {
                if result.rows_affected() == 1 {
                    Ok(())
                } else {
                    Err(PostgresError::not_found(format!(
                        "repository {repo_id} not found"
                    )))
                }
            })
    }

    pub async fn mark_git_segment_upload_ready(
        &self,
        segment: &GitSegmentRef,
        encrypted_bytes: u64,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        require_text(&segment.segment_id, "Git segment id")?;
        if segment.sha256.len() != 64
            || !segment.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(PostgresError::internal_message(
                "Git segment SHA-256 must contain 64 hexadecimal characters",
            ));
        }
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_git_segment_uploads
                 SET state = 'ready', sha256 = $2, plaintext_bytes = $3,
                     encrypted_bytes = $4,
                     updated_at_unix = GREATEST(updated_at_unix, $5)
                 WHERE segment_id = $1 AND state = 'uploading' AND encoding_version = $6",
                [
                    segment.segment_id.clone().into(),
                    segment.sha256.clone().into(),
                    u64_to_i64(segment.plaintext_bytes, "Git segment plaintext size")?.into(),
                    u64_to_i64(encrypted_bytes, "Git segment encrypted size")?.into(),
                    u64_to_i64(now_unix, "Git segment ready time")?.into(),
                    u32_to_i32(segment.encoding_version, "Git segment encoding version")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        require_one_transition(result.rows_affected(), &segment.segment_id, "ready")
    }

    pub async fn touch_git_segment_upload(
        &self,
        segment_id: &str,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        require_text(segment_id, "Git segment id")?;
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_git_segment_uploads
                 SET updated_at_unix = GREATEST(updated_at_unix, $2)
                 WHERE segment_id = $1 AND state IN ('uploading', 'ready')",
                [
                    segment_id.into(),
                    u64_to_i64(now_unix, "Git segment upload heartbeat time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn mark_git_segment_upload_published(
        &self,
        segment_id: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        transition(
            self.db.as_ref(),
            segment_id,
            "state = 'ready' AND EXISTS (
                SELECT 1 FROM scope_git_segments spans
                WHERE spans.segment_id = scope_git_segment_uploads.segment_id
            )",
            "published",
            now_unix,
        )
        .await
    }

    pub async fn mark_git_segment_upload_deleting(
        &self,
        segment_id: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        transition(
            self.db.as_ref(),
            segment_id,
            "state IN ('uploading', 'ready', 'published', 'retained') AND NOT EXISTS (
                SELECT 1 FROM scope_git_segments spans
                WHERE spans.segment_id = scope_git_segment_uploads.segment_id
            ) AND NOT EXISTS (
                SELECT 1 FROM scope_git_segment_references refs
                WHERE refs.segment_id = scope_git_segment_uploads.segment_id
            )",
            "deleting",
            now_unix,
        )
        .await
    }

    pub async fn abandon_git_segment_upload(
        &self,
        segment_id: &str,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let transitioned = transition_rows(
            self.db.as_ref(),
            segment_id,
            "state IN ('uploading', 'ready') AND NOT EXISTS (
                SELECT 1 FROM scope_git_segments spans
                WHERE spans.segment_id = scope_git_segment_uploads.segment_id
            )",
            "deleting",
            now_unix,
        )
        .await?;
        Ok(transitioned == 1)
    }

    pub async fn mark_git_segment_upload_deleted(
        &self,
        segment_id: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        transition(
            self.db.as_ref(),
            segment_id,
            "state = 'deleting'",
            "deleted",
            now_unix,
        )
        .await
    }

    pub async fn load_stale_git_segment_uploads(
        &self,
        updated_before_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitSegmentUpload>, PostgresError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        entities::git_segment_upload::Entity::find()
            .filter(entities::git_segment_upload::Column::State.is_in([
                "uploading",
                "ready",
                "deleting",
            ]))
            .filter(
                entities::git_segment_upload::Column::UpdatedAtUnix.lte(u64_to_i64(
                    updated_before_unix,
                    "Git segment recovery cutoff",
                )?),
            )
            .order_by_asc(entities::git_segment_upload::Column::UpdatedAtUnix)
            .order_by_asc(entities::git_segment_upload::Column::SegmentId)
            .limit(limit)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::git_segment_upload::Model::try_into_domain)
            .collect()
    }
}

async fn transition<C>(
    conn: &C,
    segment_id: &str,
    from_predicate: &str,
    to: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let transitioned = transition_rows(conn, segment_id, from_predicate, to, now_unix).await?;
    require_one_transition(transitioned, segment_id, to)
}

/// Moves one upload to `to` when `from_predicate` holds and reports how many
/// rows changed. `mark_git_segment_upload_ready` is the one transition that
/// also writes the segment digest and sizes, so it keeps its own statement.
async fn transition_rows<C>(
    conn: &C,
    segment_id: &str,
    from_predicate: &str,
    to: &str,
    now_unix: u64,
) -> Result<u64, PostgresError>
where
    C: ConnectionTrait,
{
    require_text(segment_id, "Git segment id")?;
    let statement = format!(
        "UPDATE scope_git_segment_uploads
         SET state = $2, updated_at_unix = GREATEST(updated_at_unix, $3)
         WHERE segment_id = $1 AND {from_predicate}"
    );
    let result = conn
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            statement,
            [
                segment_id.into(),
                to.into(),
                u64_to_i64(now_unix, "Git segment transition time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    Ok(result.rows_affected())
}

pub(super) fn require_one_transition(
    rows_affected: u64,
    segment_id: &str,
    target: &str,
) -> Result<(), PostgresError> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(PostgresError::conflict(format!(
            "Git segment {segment_id} cannot transition to {target}"
        )))
    }
}

pub(super) fn require_text(value: &str, field: &str) -> Result<(), PostgresError> {
    if value.trim().is_empty() {
        Err(PostgresError::internal_message(format!(
            "{field} is required"
        )))
    } else {
        Ok(())
    }
}
