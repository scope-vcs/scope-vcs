use super::{
    CompletedRequestAttachmentDerivative, CompletedRequestMediaManifest,
    ValidatedRequestAttachmentSource,
    access::cleanup_tombstone_exists,
    locks::lock_attachment,
    persistence::{as_i32, as_i64, attachment_by_id, enum_string},
    upload::lock_media_budget,
};
use crate::{db::locks::acquire_shared_repository_lock, error::PostgresError};
use scope_domain::requests::attachments::{
    RequestAttachment, RequestAttachmentImageMetadata, RequestAttachmentLimits,
    RequestAttachmentProcessingLease, RequestAttachmentVideoMetadata,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, Value};
use std::collections::BTreeSet;

async fn lock_live_job<C>(
    conn: &C,
    attachment_id: &str,
    lease_token: &str,
    lease_generation: u64,
    now_unix: u64,
) -> Result<Option<QueryResult>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_one(Statement::from_sql_and_values(
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
    .map_err(PostgresError::internal)
}

/// Whether a leased operation also serialises against the repository's
/// media storage budget (needed when it adds bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BudgetLock {
    Skip,
    Acquire,
}

/// Validates the worker's lease and locks the attachment it covers; `None`
/// means the lease is no longer live.
pub(super) async fn lock_lease_attachment<C>(
    conn: &C,
    attachment_id: &str,
    lease_token: &str,
    lease_generation: u64,
    now_unix: u64,
    budget: BudgetLock,
) -> Result<Option<(RequestAttachmentProcessingLease, RequestAttachment)>, PostgresError>
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
    let Some(job) =
        lock_live_job(conn, attachment_id, lease_token, lease_generation, now_unix).await?
    else {
        return Ok(None);
    };
    if cleanup_tombstone_exists(conn, attachment_id).await? {
        return Ok(None);
    }
    let attachment = lock_attachment(conn, attachment_id).await?;
    if attachment.repository_id != observed.repository_id {
        return Ok(None);
    }
    let repository_exists = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT 1 AS present FROM scope_repositories WHERE id = $1",
            [attachment.repository_id.clone().into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .is_some();
    if !repository_exists {
        return Ok(None);
    }
    let attempt = u32::try_from(
        job.try_get::<i32>("", "attempt")
            .map_err(PostgresError::internal)?,
    )
    .map_err(PostgresError::internal)?;
    let expiry = u64::try_from(
        job.try_get::<i64>("", "lease_expires_at_unix")
            .map_err(PostgresError::internal)?,
    )
    .map_err(PostgresError::internal)?;
    Ok(Some((
        RequestAttachmentProcessingLease {
            attachment_id: attachment.id.clone(),
            repository_id: attachment.repository_id.clone(),
            request_id: attachment.request_id.clone(),
            lease_token: lease_token.to_string(),
            lease_generation,
            attempt,
            lease_expires_at_unix: expiry,
        },
        attachment,
    )))
}

pub(super) fn validate_source_identity(
    attachment: &RequestAttachment,
    source: &ValidatedRequestAttachmentSource,
) -> Result<(), PostgresError> {
    if source.size_bytes != attachment.size_bytes
        || !source.sha256.eq_ignore_ascii_case(&attachment.sha256)
    {
        return Err(PostgresError::conflict(
            "validated source does not match prepared attachment",
        ));
    }
    Ok(())
}

pub(super) fn source_metadata(
    kind: scope_domain::requests::attachments::RequestAttachmentKind,
    source: &ValidatedRequestAttachmentSource,
) -> (
    Option<RequestAttachmentImageMetadata>,
    Option<RequestAttachmentVideoMetadata>,
) {
    match kind {
        scope_domain::requests::attachments::RequestAttachmentKind::Photo => (
            source
                .width
                .zip(source.height)
                .map(|(width, height)| RequestAttachmentImageMetadata { width, height }),
            None,
        ),
        scope_domain::requests::attachments::RequestAttachmentKind::Video => (
            None,
            source
                .width
                .zip(source.height)
                .zip(source.duration_millis)
                .map(
                    |((width, height), duration_millis)| RequestAttachmentVideoMetadata {
                        width,
                        height,
                        duration_millis,
                    },
                ),
        ),
    }
}

pub(super) async fn save_attachment_processing_state<C>(
    conn: &C,
    attachment: &RequestAttachment,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_attachments
         SET state = $2, failure_json = $3, original_validated_at_unix = $4,
             updated_at_unix = $5
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
            attachment
                .original_validated_at_unix
                .map(|value| as_i64(value, "original validation time"))
                .transpose()?
                .into(),
            as_i64(attachment.updated_at_unix, "attachment update time")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

/// The five nullable media-metadata columns, in table order: image width and
/// height, then video width, height, and duration.
fn media_metadata_columns(attachment: &RequestAttachment) -> Result<[Value; 5], PostgresError> {
    let image = attachment.image.as_ref();
    let video = attachment.video.as_ref();
    Ok([
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
    ])
}

pub(super) async fn save_validated_source<C>(
    conn: &C,
    attachment: &RequestAttachment,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let [
        image_width,
        image_height,
        video_width,
        video_height,
        video_duration,
    ] = media_metadata_columns(attachment)?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_attachments
         SET detected_media_type = $2, original_validated_at_unix = $3,
             image_width = $4, image_height = $5,
             video_width = $6, video_height = $7, video_duration_millis = $8,
             updated_at_unix = $9
         WHERE id = $1",
        [
            attachment.id.clone().into(),
            attachment.detected_media_type.clone().into(),
            attachment
                .original_validated_at_unix
                .map(|value| as_i64(value, "original validation time"))
                .transpose()?
                .into(),
            image_width,
            image_height,
            video_width,
            video_height,
            video_duration,
            as_i64(attachment.updated_at_unix, "attachment update time")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) fn validate_completed_derivatives(
    derivatives: &[CompletedRequestAttachmentDerivative],
) -> Result<(), PostgresError> {
    let mut derivative_ids = BTreeSet::new();
    let mut manifest_ids = BTreeSet::new();
    let mut object_keys = BTreeSet::new();
    for value in derivatives {
        if !derivative_ids.insert(value.derivative.id.as_str())
            || !manifest_ids.insert(value.manifest.id.as_str())
        {
            return Err(PostgresError::conflict(
                "request media derivative and manifest ids must be unique",
            ));
        }
        if value.derivative.object.object_key != value.manifest.id
            || value.derivative.object.size_bytes != value.manifest.size_bytes
            || value.derivative.object.sha256 != value.manifest.sha256
            || value.derivative.media_type != value.manifest.media_type
        {
            return Err(PostgresError::conflict(
                "request media derivative does not match its immutable manifest",
            ));
        }
        validate_manifest(&value.manifest, &mut object_keys)?;
    }
    Ok(())
}

fn validate_manifest(
    manifest: &CompletedRequestMediaManifest,
    object_keys: &mut BTreeSet<String>,
) -> Result<(), PostgresError> {
    if manifest.id.trim().is_empty() || manifest.media_type.trim().is_empty() {
        return Err(PostgresError::invalid_input(
            "media manifest identity is required",
        ));
    }
    if manifest.sha256.len() != 64 || !manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(PostgresError::invalid_input(
            "media manifest sha256 is invalid",
        ));
    }
    let mut offset = 0_u64;
    for (position, chunk) in manifest.chunks.iter().enumerate() {
        let expected_index = u32::try_from(position + 1)
            .map_err(|_| PostgresError::invalid_input("too many media chunks"))?;
        if chunk.index != expected_index
            || chunk.plaintext_offset != offset
            || chunk.plaintext_size_bytes == 0
            || chunk.sha256.len() != 64
            || !chunk.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !object_keys.insert(chunk.object_key.clone())
        {
            return Err(PostgresError::invalid_input(
                "media manifest chunks must be contiguous, unique, and valid",
            ));
        }
        offset = offset
            .checked_add(chunk.plaintext_size_bytes)
            .ok_or_else(|| PostgresError::invalid_input("media manifest size overflow"))?;
    }
    if manifest.chunks.is_empty() || offset != manifest.size_bytes {
        return Err(PostgresError::conflict(
            "media manifest chunks do not match its size",
        ));
    }
    Ok(())
}

pub(super) async fn ensure_manifest_keys_reserved<C>(
    conn: &C,
    attachment_id: &str,
    lease_generation: u64,
    manifest: &CompletedRequestMediaManifest,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    for chunk in &manifest.chunks {
        let reserved = conn
            .query_one(Statement::from_sql_and_values(
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
            .await
            .map_err(PostgresError::internal)?
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
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_request_media_manifests (
            id, attachment_id, derivative_id, media_type, size_bytes, sha256, completed_at_unix
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        [
            value.manifest.id.clone().into(),
            attachment_id.into(),
            value.derivative.id.clone().into(),
            value.manifest.media_type.clone().into(),
            as_i64(value.manifest.size_bytes, "derivative manifest size")?.into(),
            value.manifest.sha256.clone().into(),
            as_i64(now_unix, "derivative completion time")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    for chunk in &value.manifest.chunks {
        conn.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_request_media_manifest_chunks (
                manifest_id, chunk_index, object_key, plaintext_offset,
                plaintext_size_bytes, sha256
             ) VALUES ($1, $2, $3, $4, $5, $6)",
            [
                value.manifest.id.clone().into(),
                as_i32(chunk.index, "derivative chunk index")?.into(),
                chunk.object_key.clone().into(),
                as_i64(chunk.plaintext_offset, "derivative chunk offset")?.into(),
                as_i64(chunk.plaintext_size_bytes, "derivative chunk size")?.into(),
                chunk.sha256.clone().into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    }
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_request_media_derivatives (
            id, attachment_id, manifest_id, kind, media_type, size_bytes,
            sha256, width, height, duration_millis, created_at_unix
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
        [
            value.derivative.id.clone().into(),
            attachment_id.into(),
            value.manifest.id.clone().into(),
            enum_string(value.derivative.kind)?.into(),
            value.derivative.media_type.clone().into(),
            as_i64(value.derivative.object.size_bytes, "derivative size")?.into(),
            value.derivative.object.sha256.clone().into(),
            value
                .derivative
                .width
                .map(|value| as_i32(value, "derivative width"))
                .transpose()?
                .into(),
            value
                .derivative
                .height
                .map(|value| as_i32(value, "derivative height"))
                .transpose()?
                .into(),
            value
                .derivative
                .duration_millis
                .map(|value| as_i64(value, "derivative duration"))
                .transpose()?
                .into(),
            as_i64(now_unix, "derivative creation time")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn adopt_manifest_keys<C>(
    conn: &C,
    attachment_id: &str,
    lease_generation: u64,
    manifest: &CompletedRequestMediaManifest,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    for chunk in &manifest.chunks {
        let result = conn
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_processing_objects
                 SET state = 'Adopted', manifest_id = $4, updated_at_unix = $5
                 WHERE object_key = $1 AND attachment_id = $2
                   AND lease_generation = $3 AND state = 'Pending'",
                [
                    chunk.object_key.clone().into(),
                    attachment_id.into(),
                    as_i64(lease_generation, "processing lease generation")?.into(),
                    manifest.id.clone().into(),
                    as_i64(now_unix, "processing object adoption time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
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
    let row = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT COALESCE(SUM(
                        other.reserved_source_bytes
                        + CASE WHEN other.actual_derivative_bytes IS NULL
                               THEN other.reserved_derivative_bytes
                               ELSE other.actual_derivative_bytes END
                    ), 0)::bigint AS other_bytes
             FROM scope_request_media_attachments current
             LEFT JOIN scope_request_media_attachments other
               ON other.repository_id = current.repository_id AND other.id <> current.id
               AND NOT EXISTS (
                    SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                    WHERE cleanup.attachment_id = other.id AND cleanup.state = 'Completed'
               )
             WHERE current.id = $1
             GROUP BY current.id",
            [attachment.id.clone().into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("attachment budget row missing"))?;
    let other_bytes = u64::try_from(
        row.try_get::<i64>("", "other_bytes")
            .map_err(PostgresError::internal)?,
    )
    .map_err(PostgresError::internal)?;
    let total = other_bytes
        .checked_add(attachment.size_bytes)
        .and_then(|value| value.checked_add(actual_derivative_bytes))
        .ok_or_else(|| PostgresError::resource_exhausted("attachment storage budget overflow"))?;
    if total > RequestAttachmentLimits::default().max_repository_storage_bytes {
        return Err(PostgresError::resource_exhausted(
            "repository attachment storage budget exceeded",
        ));
    }
    Ok(())
}

pub(super) async fn save_completed_attachment<C>(
    conn: &C,
    attachment: &RequestAttachment,
    derivative_bytes: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let [
        image_width,
        image_height,
        video_width,
        video_height,
        video_duration,
    ] = media_metadata_columns(attachment)?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_attachments
         SET state = $2, detected_media_type = $3, original_validated_at_unix = $4,
             failure_json = NULL, image_width = $5, image_height = $6,
             video_width = $7, video_height = $8, video_duration_millis = $9,
             actual_derivative_bytes = $10, updated_at_unix = $11
         WHERE id = $1",
        [
            attachment.id.clone().into(),
            enum_string(attachment.state)?.into(),
            attachment.detected_media_type.clone().into(),
            attachment
                .original_validated_at_unix
                .map(|value| as_i64(value, "original validation time"))
                .transpose()?
                .into(),
            image_width,
            image_height,
            video_width,
            video_height,
            video_duration,
            as_i64(derivative_bytes, "derivative byte count")?.into(),
            as_i64(attachment.updated_at_unix, "attachment update time")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
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
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_processing_jobs
         SET state = 'Completed', lease_token = NULL, lease_expires_at_unix = NULL,
             updated_at_unix = $2 WHERE attachment_id = $1",
        [
            attachment_id.into(),
            as_i64(now_unix, "processing completion time")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}
