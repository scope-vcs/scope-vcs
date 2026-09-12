use super::{RequestMediaChunk, RequestMediaManifest};
use crate::db::entities::decode_enum;
pub(super) use crate::db::entities::encode_enum as enum_string;
use crate::db::integer_columns::{i32_to_u32 as to_u32, i64_to_u64 as to_u64};
pub(super) use crate::db::integer_columns::{u32_to_i32 as as_i32, u64_to_i64 as as_i64};
use crate::error::PostgresError;
use scope_domain::requests::attachments::{
    RequestAttachment, RequestAttachmentBinding, RequestAttachmentBindingTarget,
    RequestAttachmentDerivative, RequestAttachmentDerivativeKind, RequestAttachmentFailure,
    RequestAttachmentImageMetadata, RequestAttachmentKind, RequestAttachmentState,
    RequestAttachmentStoredObject, RequestAttachmentTarget, RequestAttachmentVideoMetadata,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, QueryResult, Statement};

#[derive(FromQueryResult)]
struct AttachmentRow {
    id: String,
    repository_id: String,
    request_id: String,
    uploader_user_id: String,
    upload_id: String,
    operation_id: String,
    target_json: serde_json::Value,
    filename: String,
    declared_media_type: String,
    detected_media_type: Option<String>,
    kind: String,
    size_bytes: i64,
    sha256: String,
    state: String,
    original_manifest_id: Option<String>,
    original_validated_at_unix: Option<i64>,
    failure_json: Option<serde_json::Value>,
    image_width: Option<i32>,
    image_height: Option<i32>,
    video_width: Option<i32>,
    video_height: Option<i32>,
    video_duration_millis: Option<i64>,
    created_at_unix: i64,
    updated_at_unix: i64,
    upload_expires_at_unix: i64,
}

#[derive(FromQueryResult)]
struct DerivativeRow {
    id: String,
    kind: String,
    media_type: String,
    manifest_id: String,
    size_bytes: i64,
    sha256: String,
    width: Option<i32>,
    height: Option<i32>,
    duration_millis: Option<i64>,
}

#[derive(FromQueryResult)]
struct ManifestRow {
    id: String,
    attachment_id: String,
    derivative_id: Option<String>,
    media_type: String,
    size_bytes: i64,
    sha256: String,
}

#[derive(FromQueryResult)]
struct ManifestChunkRow {
    chunk_index: i32,
    object_key: String,
    plaintext_offset: i64,
    plaintext_size_bytes: i64,
    sha256: String,
}

#[derive(FromQueryResult)]
struct BindingRow {
    attachment_id: String,
    request_id: String,
    target_kind: String,
    discussion_id: Option<String>,
    reply_id: Option<String>,
}

pub(super) async fn attachment_by_id<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Option<RequestAttachment>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(row) = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM scope_request_media_attachments WHERE id = $1",
            [attachment_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    Ok(Some(attachment_from_row(conn, row).await?))
}

pub(super) async fn attachment_from_row<C>(
    conn: &C,
    row: QueryResult,
) -> Result<RequestAttachment, PostgresError>
where
    C: ConnectionTrait,
{
    let row = AttachmentRow::from_query_result(&row, "").map_err(PostgresError::internal)?;
    let attachment_id = row.id;
    let original = match row.original_manifest_id {
        Some(manifest_id) => {
            let manifest = manifest_by_id(conn, &manifest_id).await?.ok_or_else(|| {
                PostgresError::internal_message("request media original manifest is missing")
            })?;
            Some(RequestAttachmentStoredObject {
                object_key: manifest.id,
                size_bytes: manifest.size_bytes,
                sha256: manifest.sha256,
            })
        }
        None => None,
    };
    let target = serde_json::from_value::<RequestAttachmentTarget>(row.target_json)
        .map_err(PostgresError::internal)?;
    let failure = row
        .failure_json
        .map(serde_json::from_value::<RequestAttachmentFailure>)
        .transpose()
        .map_err(PostgresError::internal)?;
    Ok(RequestAttachment {
        id: attachment_id.clone(),
        repository_id: row.repository_id,
        request_id: row.request_id,
        uploader_user_id: row.uploader_user_id,
        upload_id: row.upload_id,
        operation_id: row.operation_id,
        target,
        filename: row.filename,
        declared_media_type: row.declared_media_type,
        detected_media_type: row.detected_media_type,
        kind: decode_enum::<RequestAttachmentKind>(row.kind)?,
        size_bytes: to_u64(row.size_bytes, "attachment size")?,
        sha256: row.sha256,
        state: decode_enum::<RequestAttachmentState>(row.state)?,
        original,
        original_validated_at_unix: row
            .original_validated_at_unix
            .map(|value| to_u64(value, "original validation time"))
            .transpose()?,
        failure,
        image: match (row.image_width, row.image_height) {
            (Some(width), Some(height)) => Some(RequestAttachmentImageMetadata {
                width: to_u32(width, "image width")?,
                height: to_u32(height, "image height")?,
            }),
            (None, None) => None,
            _ => {
                return Err(PostgresError::internal_message(
                    "request media image metadata is incomplete",
                ));
            }
        },
        video: match (row.video_width, row.video_height, row.video_duration_millis) {
            (Some(width), Some(height), Some(duration_millis)) => {
                Some(RequestAttachmentVideoMetadata {
                    width: to_u32(width, "video width")?,
                    height: to_u32(height, "video height")?,
                    duration_millis: to_u64(duration_millis, "video duration")?,
                })
            }
            (None, None, None) => None,
            _ => {
                return Err(PostgresError::internal_message(
                    "request media video metadata is incomplete",
                ));
            }
        },
        derivatives: derivatives_for_attachment(conn, &attachment_id).await?,
        created_at_unix: to_u64(row.created_at_unix, "attachment creation time")?,
        updated_at_unix: to_u64(row.updated_at_unix, "attachment update time")?,
        upload_expires_at_unix: to_u64(row.upload_expires_at_unix, "upload expiry")?,
    })
}

async fn derivatives_for_attachment<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Vec<RequestAttachmentDerivative>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_all(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT * FROM scope_request_media_derivatives WHERE attachment_id = $1 ORDER BY id",
        [attachment_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)?
    .into_iter()
    .map(|row| {
        let row = DerivativeRow::from_query_result(&row, "").map_err(PostgresError::internal)?;
        let width = row
            .width
            .map(|value| to_u32(value, "derivative width"))
            .transpose()?;
        let height = row
            .height
            .map(|value| to_u32(value, "derivative height"))
            .transpose()?;
        let duration_millis = row
            .duration_millis
            .map(|value| to_u64(value, "derivative duration"))
            .transpose()?;
        Ok(RequestAttachmentDerivative {
            id: row.id,
            kind: decode_enum::<RequestAttachmentDerivativeKind>(row.kind)?,
            media_type: row.media_type,
            object: RequestAttachmentStoredObject {
                object_key: row.manifest_id,
                size_bytes: to_u64(row.size_bytes, "derivative size")?,
                sha256: row.sha256,
            },
            width,
            height,
            duration_millis,
        })
    })
    .collect()
}

pub(super) async fn manifest_by_id<C>(
    conn: &C,
    manifest_id: &str,
) -> Result<Option<RequestMediaManifest>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(row) = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM scope_request_media_manifests WHERE id = $1",
            [manifest_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    let row = ManifestRow::from_query_result(&row, "").map_err(PostgresError::internal)?;
    let chunks = conn
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM scope_request_media_manifest_chunks WHERE manifest_id = $1 ORDER BY chunk_index",
            [manifest_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|chunk| {
            let chunk = ManifestChunkRow::from_query_result(&chunk, "").map_err(PostgresError::internal)?;
            Ok(RequestMediaChunk {
                index: to_u32(
                    chunk.chunk_index,
                    "manifest chunk index",
                )?,
                object_key: chunk.object_key,
                plaintext_offset: to_u64(
                    chunk.plaintext_offset,
                    "manifest chunk offset",
                )?,
                plaintext_size_bytes: to_u64(
                    chunk.plaintext_size_bytes,
                    "manifest chunk size",
                )?,
                sha256: chunk.sha256,
            })
        })
        .collect::<Result<Vec<_>, PostgresError>>()?;
    Ok(Some(RequestMediaManifest {
        id: row.id,
        attachment_id: row.attachment_id,
        derivative_id: row.derivative_id,
        media_type: row.media_type,
        size_bytes: to_u64(row.size_bytes, "manifest size")?,
        sha256: row.sha256,
        chunks,
    }))
}

pub(super) async fn bindings_for_attachment<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Vec<RequestAttachmentBinding>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_all(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT * FROM scope_request_media_bindings WHERE attachment_id = $1 ORDER BY target_key",
        [attachment_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)?
    .into_iter()
    .map(|row| {
        let row = BindingRow::from_query_result(&row, "").map_err(PostgresError::internal)?;
        let target = match (row.target_kind.as_str(), row.discussion_id, row.reply_id) {
            ("Description", None, None) => RequestAttachmentBindingTarget::Description,
            ("Discussion", Some(discussion_id), None) => {
                RequestAttachmentBindingTarget::Discussion { discussion_id }
            }
            ("Reply", Some(discussion_id), Some(reply_id)) => {
                RequestAttachmentBindingTarget::Reply {
                    discussion_id,
                    reply_id,
                }
            }
            _ => {
                return Err(PostgresError::internal_message(
                    "request media binding target is invalid",
                ));
            }
        };
        Ok(RequestAttachmentBinding {
            attachment_id: row.attachment_id,
            request_id: row.request_id,
            target,
        })
    })
    .collect()
}

pub(super) fn binding_target_parts(
    target: &RequestAttachmentBindingTarget,
) -> (String, &'static str, Option<String>, Option<String>) {
    match target {
        RequestAttachmentBindingTarget::Description => {
            ("description".to_string(), "Description", None, None)
        }
        RequestAttachmentBindingTarget::Discussion { discussion_id } => (
            format!("discussion:{discussion_id}"),
            "Discussion",
            Some(discussion_id.clone()),
            None,
        ),
        RequestAttachmentBindingTarget::Reply {
            discussion_id,
            reply_id,
        } => (
            format!("reply:{reply_id}"),
            "Reply",
            Some(discussion_id.clone()),
            Some(reply_id.clone()),
        ),
    }
}
