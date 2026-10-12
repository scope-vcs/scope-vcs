use super::{
    CompletedRequestAttachmentDerivative,
    access::cleanup_tombstone_exists,
    budget::{lock_media_budget, media_usage, storage_limit},
    locks::lock_attachment,
    persistence::{as_i32, as_i64, attachment_by_id, enum_string, insert_manifest},
};
use crate::{db::locks::acquire_shared_repository_lock, error::PostgresError};
use scope_domain::requests::attachments::{
    RequestAttachment, RequestMediaChunk, RequestMediaObject,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

async fn lock_live_job<C>(
    conn: &C,
    attachment_id: &str,
    lease_token: &str,
    lease_generation: u64,
    now_unix: u64,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    Ok(conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM scope_request_media_processing_jobs
         WHERE attachment_id = $1 AND state = 'Leased' AND lease_token = $2
           AND lease_generation = $3 AND lease_expires_at_unix > $4
         FOR UPDATE",
            [
                attachment_id.into(),
                lease_token.into(),
                as_i64(lease_generation, "processing lease generation")?.into(),
                as_i64(now_unix, "processing lease operation time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?
        .is_some())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BudgetLock {
    Skip,
    Acquire,
}

pub(super) async fn lock_lease_attachment<C>(
    conn: &C,
    attachment_id: &str,
    lease_token: &str,
    lease_generation: u64,
    now_unix: u64,
    budget: BudgetLock,
) -> Result<Option<RequestAttachment>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(observed) = attachment_by_id(conn, attachment_id).await? else {
        return Ok(None);
    };
    acquire_shared_repository_lock(conn, &observed.repository_id).await?;
    if budget == BudgetLock::Acquire {
        lock_media_budget(conn, &observed.repository_id).await?;
    }
    if !lock_live_job(conn, attachment_id, lease_token, lease_generation, now_unix).await?
        || cleanup_tombstone_exists(conn, attachment_id).await?
    {
        return Ok(None);
    }
    let attachment = lock_attachment(conn, attachment_id).await?;
    if attachment.repository_id != observed.repository_id {
        return Ok(None);
    }
    let repository_exists = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT 1 AS present FROM scope_repositories WHERE id = $1",
            [attachment.repository_id.clone().into()],
        ))
        .await?
        .is_some();
    Ok(repository_exists.then_some(attachment))
}

pub(super) async fn save_attachment<C>(
    conn: &C,
    attachment: &RequestAttachment,
    actual_derivative_bytes: Option<u64>,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let image = attachment.image.as_ref();
    let video = attachment.video.as_ref();
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_attachments
         SET state = $2, failure_json = $3, detected_media_type = $4,
             original_validated_at_unix = $5, image_width = $6, image_height = $7,
             video_width = $8, video_height = $9, video_duration_millis = $10,
             actual_derivative_bytes = COALESCE($11, actual_derivative_bytes),
             updated_at_unix = $12
         WHERE id = $1",
        [
            attachment.id.clone().into(),
            enum_string(attachment.state)?.into(),
            attachment
                .failure
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(PostgresError::internal)?
                .into(),
            attachment.detected_media_type.clone().into(),
            attachment
                .original_validated_at_unix
                .map(|value| as_i64(value, "original validation time"))
                .transpose()?
                .into(),
            image
                .map(|image| as_i32(image.width, "image width"))
                .transpose()?
                .into(),
            image
                .map(|image| as_i32(image.height, "image height"))
                .transpose()?
                .into(),
            video
                .map(|video| as_i32(video.width, "video width"))
                .transpose()?
                .into(),
            video
                .map(|video| as_i32(video.height, "video height"))
                .transpose()?
                .into(),
            video
                .map(|video| as_i64(video.duration_millis, "video duration"))
                .transpose()?
                .into(),
            actual_derivative_bytes
                .map(|value| as_i64(value, "derivative byte count"))
                .transpose()?
                .into(),
            as_i64(attachment.updated_at_unix, "attachment update time")?.into(),
        ],
    ))
    .await?;
    Ok(())
}

pub(super) fn derivative_object(
    value: &CompletedRequestAttachmentDerivative,
) -> RequestMediaObject {
    RequestMediaObject {
        media_type: value.derivative.media_type.clone(),
        plaintext_bytes: value.derivative.object.size_bytes,
        sha256: value.derivative.object.sha256.clone(),
        chunks: value.chunks.clone(),
    }
}

pub(super) async fn ensure_manifest_keys_reserved<C>(
    conn: &C,
    attachment_id: &str,
    lease_generation: u64,
    chunks: &[RequestMediaChunk],
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    for chunk in chunks {
        let reserved = conn
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT 1 AS present FROM scope_request_media_processing_objects
                 WHERE object_key = $1 AND attachment_id = $2
                   AND lease_generation = $3 AND state = 'Pending'
                 FOR UPDATE",
                [
                    chunk.object_key.clone().into(),
                    attachment_id.into(),
                    as_i64(lease_generation, "processing lease generation")?.into(),
                ],
            ))
            .await?
            .is_some();
        if !reserved {
            return Err(PostgresError::conflict(
                "media manifest contains an unreserved processing object",
            ));
        }
    }
    Ok(())
}

pub(super) async fn insert_derivative<C>(
    conn: &C,
    attachment_id: &str,
    value: &CompletedRequestAttachmentDerivative,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let derivative = &value.derivative;
    insert_manifest(
        conn,
        &derivative.object.object_key,
        attachment_id,
        Some(&derivative.id),
        &derivative_object(value),
        now_unix,
    )
    .await?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_request_media_derivatives (
            id, attachment_id, manifest_id, kind, media_type, size_bytes,
            sha256, width, height, duration_millis, created_at_unix
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        [
            derivative.id.clone().into(),
            attachment_id.into(),
            derivative.object.object_key.clone().into(),
            enum_string(derivative.kind)?.into(),
            derivative.media_type.clone().into(),
            as_i64(derivative.object.size_bytes, "derivative size")?.into(),
            derivative.object.sha256.clone().into(),
            derivative
                .width
                .map(|value| as_i32(value, "derivative width"))
                .transpose()?
                .into(),
            derivative
                .height
                .map(|value| as_i32(value, "derivative height"))
                .transpose()?
                .into(),
            derivative
                .duration_millis
                .map(|value| as_i64(value, "derivative duration"))
                .transpose()?
                .into(),
            as_i64(now_unix, "derivative creation time")?.into(),
        ],
    ))
    .await?;
    Ok(())
}

pub(super) async fn adopt_manifest_keys<C>(
    conn: &C,
    attachment_id: &str,
    lease_generation: u64,
    manifest_id: &str,
    chunks: &[RequestMediaChunk],
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    for chunk in chunks {
        let result = conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_processing_objects
                 SET state = 'Adopted', manifest_id = $4, updated_at_unix = $5
                 WHERE object_key = $1 AND attachment_id = $2
                   AND lease_generation = $3 AND state = 'Pending'",
                [
                    chunk.object_key.clone().into(),
                    attachment_id.into(),
                    as_i64(lease_generation, "processing lease generation")?.into(),
                    manifest_id.into(),
                    as_i64(now_unix, "processing object adoption time")?.into(),
                ],
            ))
            .await?;
        if result.rows_affected() != 1 {
            return Err(PostgresError::conflict(
                "processing object reservation changed before completion",
            ));
        }
    }
    Ok(())
}

pub(super) async fn ensure_derivative_budget<C>(
    conn: &C,
    attachment: &RequestAttachment,
    actual_derivative_bytes: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let other_bytes = media_usage(conn, &attachment.repository_id, None, Some(&attachment.id))
        .await?
        .repository_bytes;
    let total = other_bytes
        .checked_add(attachment.size_bytes)
        .and_then(|value| value.checked_add(actual_derivative_bytes))
        .ok_or_else(|| PostgresError::resource_exhausted("attachment storage budget overflow"))?;
    if total > storage_limit() {
        return Err(PostgresError::resource_exhausted(
            "repository attachment storage budget exceeded",
        ));
    }
    Ok(())
}

pub(super) async fn complete_job<C>(
    conn: &C,
    attachment_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_processing_jobs
         SET state = 'Completed', lease_token = NULL, lease_expires_at_unix = NULL,
             updated_at_unix = $2 WHERE attachment_id = $1",
        [
            attachment_id.into(),
            as_i64(now_unix, "processing completion time")?.into(),
        ],
    ))
    .await?;
    Ok(())
}
