use crate::db::entities::i64_to_u64;
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

pub(super) struct MediaUsage {
    pub request_source_bytes: u64,
    pub repository_bytes: u64,
}

// A completed deletion releases its reservation permanently. Reconciliation may
// reopen the cleanup job, but it must never reserve these bytes again.
pub(super) async fn media_usage<C: ConnectionTrait>(
    conn: &C,
    repository_id: &str,
    request_id: Option<&str>,
    exclude_attachment_id: Option<&str>,
) -> Result<MediaUsage, PostgresError> {
    let row = conn.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT COALESCE(SUM(reserved_source_bytes) FILTER (WHERE request_id = $2), 0)::bigint AS source_bytes,
                COALESCE(SUM(reserved_source_bytes + COALESCE(actual_derivative_bytes, reserved_derivative_bytes)), 0)::bigint AS total_bytes
         FROM scope_request_media_attachments
         WHERE repository_id = $1 AND budget_released_at_unix IS NULL
           AND ($3::text IS NULL OR id <> $3)",
        [repository_id.into(), request_id.map(str::to_owned).into(), exclude_attachment_id.map(str::to_owned).into()],
    ))
    .await
    .map_err(PostgresError::internal)?
    .ok_or_else(|| PostgresError::internal_message("attachment usage missing"))?;
    Ok(MediaUsage {
        request_source_bytes: i64_to_u64(
            row.try_get("", "source_bytes")
                .map_err(PostgresError::internal)?,
            "media source usage",
        )?,
        repository_bytes: i64_to_u64(
            row.try_get("", "total_bytes")
                .map_err(PostgresError::internal)?,
            "media repository usage",
        )?,
    })
}

pub(super) async fn lock_media_budget<C: ConnectionTrait>(
    conn: &C,
    repository_id: &str,
) -> Result<(), PostgresError> {
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT pg_advisory_xact_lock(hashtextextended('scope:request-media-budget:' || $1, 0))",
        [repository_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}
