use super::{RequestMediaChunk, RequestMediaManifest};
use crate::error::PostgresError;
use scope_domain::requests::attachments::{
    RequestAttachment, RequestAttachmentBinding, RequestAttachmentBindingTarget,
    RequestAttachmentDerivative, RequestAttachmentDerivativeKind, RequestAttachmentFailure,
    RequestAttachmentImageMetadata, RequestAttachmentKind, RequestAttachmentState,
    RequestAttachmentStoredObject, RequestAttachmentTarget, RequestAttachmentVideoMetadata,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement};
use serde::{Serialize, de::DeserializeOwned};

pub(super) fn enum_string<T: Serialize>(value: T) -> Result<String, PostgresError> {
    match serde_json::to_value(value).map_err(PostgresError::internal)? {
        serde_json::Value::String(value) => Ok(value),
        _ => Err(PostgresError::internal_message(
            "request media enum did not serialize to a string",
        )),
    }
}

fn decode_enum<T: DeserializeOwned>(value: String) -> Result<T, PostgresError> {
    serde_json::from_value(serde_json::Value::String(value)).map_err(PostgresError::internal)
}

fn to_u64(value: i64, field: &str) -> Result<u64, PostgresError> {
    u64::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{field} cannot be negative")))
}

fn to_u32(value: i32, field: &str) -> Result<u32, PostgresError> {
    u32::try_from(value)
        .map_err(|_| PostgresError::internal_message(format!("{field} cannot be negative")))
}

pub(super) fn as_i64(value: u64, field: &str) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(|_| {
        PostgresError::internal_message(format!("{field} exceeds PostgreSQL bigint range"))
    })
}

pub(super) fn as_i32(value: u32, field: &str) -> Result<i32, PostgresError> {
    i32::try_from(value).map_err(|_| {
        PostgresError::internal_message(format!("{field} exceeds PostgreSQL integer range"))
    })
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
    let attachment_id = row
        .try_get::<String>("", "id")
        .map_err(PostgresError::internal)?;
    let original_manifest_id = row
        .try_get::<Option<String>>("", "original_manifest_id")
        .map_err(PostgresError::internal)?;
    let original = match original_manifest_id {
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
    let image_width = row
        .try_get::<Option<i32>>("", "image_width")
        .map_err(PostgresError::internal)?;
    let image_height = row
        .try_get::<Option<i32>>("", "image_height")
        .map_err(PostgresError::internal)?;
    let video_width = row
        .try_get::<Option<i32>>("", "video_width")
        .map_err(PostgresError::internal)?;
    let video_height = row
        .try_get::<Option<i32>>("", "video_height")
        .map_err(PostgresError::internal)?;
    let duration = row
        .try_get::<Option<i64>>("", "video_duration_millis")
        .map_err(PostgresError::internal)?;
    let target = serde_json::from_value::<RequestAttachmentTarget>(
        row.try_get::<serde_json::Value>("", "target_json")
            .map_err(PostgresError::internal)?,
    )
    .map_err(PostgresError::internal)?;
    let failure = row
        .try_get::<Option<serde_json::Value>>("", "failure_json")
        .map_err(PostgresError::internal)?
        .map(serde_json::from_value::<RequestAttachmentFailure>)
        .transpose()
        .map_err(PostgresError::internal)?;
    Ok(RequestAttachment {
        id: attachment_id.clone(),
        repository_id: row
            .try_get("", "repository_id")
            .map_err(PostgresError::internal)?,
        request_id: row
            .try_get("", "request_id")
            .map_err(PostgresError::internal)?,
        uploader_user_id: row
            .try_get("", "uploader_user_id")
            .map_err(PostgresError::internal)?,
        upload_id: row
            .try_get("", "upload_id")
            .map_err(PostgresError::internal)?,
        operation_id: row
            .try_get("", "operation_id")
            .map_err(PostgresError::internal)?,
        target,
        filename: row
            .try_get("", "filename")
            .map_err(PostgresError::internal)?,
        declared_media_type: row
            .try_get("", "declared_media_type")
            .map_err(PostgresError::internal)?,
        detected_media_type: row
            .try_get("", "detected_media_type")
            .map_err(PostgresError::internal)?,
        kind: decode_enum::<RequestAttachmentKind>(
            row.try_get("", "kind").map_err(PostgresError::internal)?,
        )?,
        size_bytes: to_u64(
            row.try_get("", "size_bytes")
                .map_err(PostgresError::internal)?,
            "attachment size",
        )?,
        sha256: row.try_get("", "sha256").map_err(PostgresError::internal)?,
        state: decode_enum::<RequestAttachmentState>(
            row.try_get("", "state").map_err(PostgresError::internal)?,
        )?,
        original,
        original_validated_at_unix: row
            .try_get::<Option<i64>>("", "original_validated_at_unix")
            .map_err(PostgresError::internal)?
            .map(|value| to_u64(value, "original validation time"))
            .transpose()?,
        failure,
        image: match (image_width, image_height) {
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
        video: match (video_width, video_height, duration) {
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
        created_at_unix: to_u64(
            row.try_get("", "created_at_unix")
                .map_err(PostgresError::internal)?,
            "attachment creation time",
        )?,
        updated_at_unix: to_u64(
            row.try_get("", "updated_at_unix")
                .map_err(PostgresError::internal)?,
            "attachment update time",
        )?,
        upload_expires_at_unix: to_u64(
            row.try_get("", "upload_expires_at_unix")
                .map_err(PostgresError::internal)?,
            "upload expiry",
        )?,
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
        let width = row
            .try_get::<Option<i32>>("", "width")
            .map_err(PostgresError::internal)?
            .map(|value| to_u32(value, "derivative width"))
            .transpose()?;
        let height = row
            .try_get::<Option<i32>>("", "height")
            .map_err(PostgresError::internal)?
            .map(|value| to_u32(value, "derivative height"))
            .transpose()?;
        let duration_millis = row
            .try_get::<Option<i64>>("", "duration_millis")
            .map_err(PostgresError::internal)?
            .map(|value| to_u64(value, "derivative duration"))
            .transpose()?;
        Ok(RequestAttachmentDerivative {
            id: row.try_get("", "id").map_err(PostgresError::internal)?,
            kind: decode_enum::<RequestAttachmentDerivativeKind>(
                row.try_get("", "kind").map_err(PostgresError::internal)?,
            )?,
            media_type: row
                .try_get("", "media_type")
                .map_err(PostgresError::internal)?,
            object: RequestAttachmentStoredObject {
                object_key: row
                    .try_get("", "manifest_id")
                    .map_err(PostgresError::internal)?,
                size_bytes: to_u64(
                    row.try_get("", "size_bytes")
                        .map_err(PostgresError::internal)?,
                    "derivative size",
                )?,
                sha256: row.try_get("", "sha256").map_err(PostgresError::internal)?,
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
            Ok(RequestMediaChunk {
                index: to_u32(
                    chunk
                        .try_get::<i32>("", "chunk_index")
                        .map_err(PostgresError::internal)?,
                    "manifest chunk index",
                )?,
                object_key: chunk
                    .try_get("", "object_key")
                    .map_err(PostgresError::internal)?,
                plaintext_offset: to_u64(
                    chunk
                        .try_get("", "plaintext_offset")
                        .map_err(PostgresError::internal)?,
                    "manifest chunk offset",
                )?,
                plaintext_size_bytes: to_u64(
                    chunk
                        .try_get("", "plaintext_size_bytes")
                        .map_err(PostgresError::internal)?,
                    "manifest chunk size",
                )?,
                sha256: chunk
                    .try_get("", "sha256")
                    .map_err(PostgresError::internal)?,
            })
        })
        .collect::<Result<Vec<_>, PostgresError>>()?;
    Ok(Some(RequestMediaManifest {
        id: row.try_get("", "id").map_err(PostgresError::internal)?,
        attachment_id: row
            .try_get("", "attachment_id")
            .map_err(PostgresError::internal)?,
        derivative_id: row
            .try_get("", "derivative_id")
            .map_err(PostgresError::internal)?,
        media_type: row
            .try_get("", "media_type")
            .map_err(PostgresError::internal)?,
        size_bytes: to_u64(
            row.try_get("", "size_bytes")
                .map_err(PostgresError::internal)?,
            "manifest size",
        )?,
        sha256: row.try_get("", "sha256").map_err(PostgresError::internal)?,
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
        let target_kind = row
            .try_get::<String>("", "target_kind")
            .map_err(PostgresError::internal)?;
        let discussion_id = row
            .try_get::<Option<String>>("", "discussion_id")
            .map_err(PostgresError::internal)?;
        let reply_id = row
            .try_get::<Option<String>>("", "reply_id")
            .map_err(PostgresError::internal)?;
        let target = match (target_kind.as_str(), discussion_id, reply_id) {
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
            attachment_id: row
                .try_get("", "attachment_id")
                .map_err(PostgresError::internal)?,
            request_id: row
                .try_get("", "request_id")
                .map_err(PostgresError::internal)?,
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
