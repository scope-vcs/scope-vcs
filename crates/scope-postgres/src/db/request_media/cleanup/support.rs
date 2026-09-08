use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement};

use super::{LATE_WRITE_GRACE_SECONDS, TOMBSTONE_RECONCILIATION_SECONDS};
use crate::db::request_media::persistence::as_i64;

const RECONCILIATION_BATCH_SIZE: i64 = 256;

pub(super) async fn all_attachment_object_keys<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Vec<String>, PostgresError>
where
    C: ConnectionTrait,
{
    let rows = conn
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT object_key FROM scope_request_media_upload_parts WHERE attachment_id = $1
             UNION SELECT object_key FROM scope_request_media_abandoned_objects
                   WHERE attachment_id = $1
             UNION SELECT chunk.object_key FROM scope_request_media_manifest_chunks chunk
                   JOIN scope_request_media_manifests manifest ON manifest.id = chunk.manifest_id
                   WHERE manifest.attachment_id = $1
             UNION SELECT object_key FROM scope_request_media_processing_objects
                   WHERE attachment_id = $1
             ORDER BY object_key",
            [attachment_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    rows.into_iter()
        .map(|row| {
            row.try_get::<String>("", "object_key")
                .map_err(PostgresError::internal)
        })
        .collect()
}

pub(super) async fn orphan_object_keys<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Vec<String>, PostgresError>
where
    C: ConnectionTrait,
{
    let rows = conn
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT object_key FROM scope_request_media_abandoned_objects
             WHERE attachment_id = $1 AND deleted_at_unix IS NULL
             UNION SELECT object_key FROM scope_request_media_processing_objects
             WHERE attachment_id = $1 AND state = 'Orphaned'
             ORDER BY object_key",
            [attachment_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    rows.into_iter()
        .map(|row| {
            row.try_get::<String>("", "object_key")
                .map_err(PostgresError::internal)
        })
        .collect()
}

pub(super) async fn mark_all_inventory_deleted<C>(
    conn: &C,
    attachment_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let now = as_i64(now_unix, "inventory deletion time")?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_abandoned_objects SET deleted_at_unix = $2
         WHERE attachment_id = $1",
        [attachment_id.into(), now.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_processing_objects
         SET state = 'Deleted', updated_at_unix = $2 WHERE attachment_id = $1",
        [attachment_id.into(), now.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn mark_orphan_inventory_deleted<C>(
    conn: &C,
    attachment_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let now = as_i64(now_unix, "orphan deletion time")?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_abandoned_objects SET deleted_at_unix = $2
         WHERE attachment_id = $1 AND deleted_at_unix IS NULL",
        [attachment_id.into(), now.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_processing_objects
         SET state = 'Deleted', updated_at_unix = $2
         WHERE attachment_id = $1 AND state = 'Orphaned'",
        [attachment_id.into(), now.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn reconcile_completed_inventories<C>(
    conn: &C,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let cutoff = now_unix.saturating_sub(TOMBSTONE_RECONCILIATION_SECONDS);
    let now = as_i64(now_unix, "inventory reconciliation time")?;
    let cutoff = as_i64(cutoff, "inventory reconciliation cutoff")?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "WITH due AS (
            SELECT attachment_id FROM scope_request_media_cleanup_jobs
            WHERE state = 'Completed' AND completed_at_unix <= $2
            ORDER BY completed_at_unix, attachment_id
            FOR UPDATE SKIP LOCKED LIMIT $3
         )
         UPDATE scope_request_media_cleanup_jobs cleanup
         SET state = 'Queued', available_at_unix = $1, completed_at_unix = NULL,
             updated_at_unix = $1
         FROM due WHERE cleanup.attachment_id = due.attachment_id",
        [now.into(), cutoff.into(), RECONCILIATION_BATCH_SIZE.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "WITH due AS (
            SELECT abandoned.object_key
            FROM scope_request_media_abandoned_objects abandoned
            WHERE abandoned.deleted_at_unix IS NOT NULL
              AND abandoned.deleted_at_unix <= $1
              AND NOT EXISTS (
                    SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                    WHERE cleanup.attachment_id = abandoned.attachment_id
            )
            ORDER BY abandoned.deleted_at_unix, abandoned.object_key
            FOR UPDATE OF abandoned SKIP LOCKED LIMIT $2
         )
         UPDATE scope_request_media_abandoned_objects abandoned
         SET deleted_at_unix = NULL
         FROM due WHERE abandoned.object_key = due.object_key",
        [cutoff.into(), RECONCILIATION_BATCH_SIZE.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "WITH due AS (
            SELECT object.object_key
            FROM scope_request_media_processing_objects object
            WHERE object.state = 'Deleted' AND object.updated_at_unix <= $2
              AND object.manifest_id IS NULL
              AND NOT EXISTS (
                    SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                    WHERE cleanup.attachment_id = object.attachment_id
            )
            ORDER BY object.updated_at_unix, object.object_key
            FOR UPDATE OF object SKIP LOCKED LIMIT $3
         )
         UPDATE scope_request_media_processing_objects object
         SET state = 'Orphaned', updated_at_unix = $1
         FROM due WHERE object.object_key = due.object_key",
        [now.into(), cutoff.into(), RECONCILIATION_BATCH_SIZE.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn cleanup_available_at<C>(
    conn: &C,
    attachment_id: &str,
    now_unix: u64,
) -> Result<u64, PostgresError>
where
    C: ConnectionTrait,
{
    let max_expiry = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT MAX(active.expires_at_unix) AS expires
             FROM (
                SELECT write_expires_at_unix AS expires_at_unix
                FROM scope_request_media_upload_parts
                WHERE attachment_id = $1 AND state = 'Pending'
                UNION ALL
                SELECT lease_expires_at_unix AS expires_at_unix
                FROM scope_request_media_processing_jobs
                WHERE attachment_id = $1 AND state = 'Leased'
             ) active",
            [attachment_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("upload write lease query missing"))?
        .try_get::<Option<i64>>("", "expires")
        .map_err(PostgresError::internal)?
        .map(u64::try_from)
        .transpose()
        .map_err(PostgresError::internal)?;
    Ok(max_expiry
        .and_then(|expiry| expiry.checked_add(LATE_WRITE_GRACE_SECONDS))
        .map_or(now_unix, |expiry| expiry.max(now_unix)))
}

pub(super) async fn attachment_cleanup_is_due<C>(
    conn: &C,
    row: &QueryResult,
    now_unix: u64,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    let attachment_id = row
        .try_get::<String>("", "id")
        .map_err(PostgresError::internal)?;
    let state = row
        .try_get::<String>("", "state")
        .map_err(PostgresError::internal)?;
    if state == "Prepared" {
        let expiry = u64::try_from(
            row.try_get::<i64>("", "upload_expires_at_unix")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?;
        return Ok(expiry <= now_unix);
    }
    let expiry = row
        .try_get::<Option<i64>>("", "unbound_expires_at_unix")
        .map_err(PostgresError::internal)?
        .map(u64::try_from)
        .transpose()
        .map_err(PostgresError::internal)?;
    if expiry.is_none_or(|expiry| expiry > now_unix) {
        return Ok(false);
    }
    Ok(conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT 1 AS present FROM scope_request_media_bindings
             WHERE attachment_id = $1 LIMIT 1",
            [attachment_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .is_none())
}

pub(super) async fn lock_attachment_row<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Option<QueryResult>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT * FROM scope_request_media_attachments WHERE id = $1 FOR UPDATE",
        [attachment_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)
}

pub(super) async fn lock_processing_job_if_present<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT attachment_id FROM scope_request_media_processing_jobs
         WHERE attachment_id = $1 FOR UPDATE",
        [attachment_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn cancel_processing_and_orphan_outputs<C>(
    conn: &C,
    attachment_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let now = as_i64(now_unix, "attachment tombstone time")?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_processing_jobs
         SET state = 'Canceled', lease_token = NULL, lease_expires_at_unix = NULL,
             updated_at_unix = $2 WHERE attachment_id = $1 AND state <> 'Completed'",
        [attachment_id.into(), now.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_processing_objects
         SET state = 'Orphaned', updated_at_unix = $2
         WHERE attachment_id = $1 AND state = 'Pending'",
        [attachment_id.into(), now.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}
