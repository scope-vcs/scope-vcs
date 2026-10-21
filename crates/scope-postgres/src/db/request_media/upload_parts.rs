use super::{
    MediaStore, ReserveUploadPartResult, StorePartResult,
    persistence::{as_i32, as_i64},
    upload::{authorize_active_upload, lock_authorized_upload},
};
use crate::error::PostgresError;
use scope_domain::requests::attachments::{
    RequestAttachmentPartReceipt, RequestAttachmentStagedPart, validate_lease_grant,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, TransactionTrait};

impl MediaStore {
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "reserve_upload_part"))]
    pub async fn reserve_upload_part(
        &self,
        attachment_id: &str,
        upload_id: &str,
        uploader_user_id: &str,
        part: RequestAttachmentStagedPart,
        write_token: &str,
        now_unix: u64,
        write_expires_at_unix: u64,
    ) -> Result<ReserveUploadPartResult, PostgresError> {
        validate_lease_grant(write_token, now_unix, write_expires_at_unix)?;
        let tx = self.db.begin().await?;
        let attachment = lock_authorized_upload(&tx, attachment_id, uploader_user_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request attachment upload not found"))?;
        authorize_active_upload(&attachment, upload_id, uploader_user_id, now_unix)?;
        scope_domain::requests::attachments::validate_attachment_part(&attachment, &part.receipt)?;
        if part.object_key.trim().is_empty() {
            return Err(PostgresError::invalid_input("media object key is required"));
        }
        if let Some(existing) = part_by_number(&tx, attachment_id, part.receipt.part_number).await?
        {
            if existing.part.receipt != part.receipt {
                return Err(PostgresError::conflict(
                    "upload part number is already reserved for different bytes",
                ));
            }
            if existing.stored {
                tx.commit().await?;
                return Ok(ReserveUploadPartResult::Stored(existing.part));
            }
            if existing
                .write_expires_at_unix
                .is_some_and(|expires| expires > now_unix)
            {
                tx.commit().await?;
                return Ok(ReserveUploadPartResult::Busy);
            }
            if existing.part.object_key == part.object_key {
                return Err(PostgresError::conflict(
                    "expired upload part retries require a fresh object key",
                ));
            }
            tx.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_request_media_abandoned_objects (
                    object_key, attachment_id, created_at_unix
                 ) VALUES ($1, $2, $3) ON CONFLICT (object_key) DO NOTHING",
                [
                    existing.part.object_key.into(),
                    attachment_id.into(),
                    as_i64(now_unix, "abandoned upload object time")?.into(),
                ],
            ))
            .await?;
            tx.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_upload_parts
                 SET object_key = $3, write_token = $4, write_expires_at_unix = $5,
                     created_at_unix = $6, stored_at_unix = NULL
                 WHERE attachment_id = $1 AND part_number = $2 AND state = 'Pending'",
                [
                    attachment_id.into(),
                    as_i32(part.receipt.part_number, "part number")?.into(),
                    part.object_key.clone().into(),
                    write_token.into(),
                    as_i64(write_expires_at_unix, "upload write expiry")?.into(),
                    as_i64(now_unix, "part reservation time")?.into(),
                ],
            ))
            .await?;
            tx.commit().await?;
            return Ok(ReserveUploadPartResult::Write(part));
        }
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_request_media_upload_parts (
                attachment_id, upload_id, part_number, plaintext_size_bytes,
                sha256, object_key, state, write_token, write_expires_at_unix,
                created_at_unix
             ) VALUES ($1, $2, $3, $4, $5, $6, 'Pending', $7, $8, $9)",
            [
                attachment_id.into(),
                upload_id.into(),
                as_i32(part.receipt.part_number, "part number")?.into(),
                as_i64(part.receipt.size_bytes, "part size")?.into(),
                part.receipt.sha256.to_ascii_lowercase().into(),
                part.object_key.clone().into(),
                write_token.into(),
                as_i64(write_expires_at_unix, "upload write expiry")?.into(),
                as_i64(now_unix, "part reservation time")?.into(),
            ],
        ))
        .await?;
        tx.commit().await?;
        Ok(ReserveUploadPartResult::Write(part))
    }

    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "mark_upload_part_stored"))]
    pub async fn mark_upload_part_stored(
        &self,
        attachment_id: &str,
        upload_id: &str,
        uploader_user_id: &str,
        part_number: u32,
        object_key: &str,
        write_token: &str,
        now_unix: u64,
    ) -> Result<StorePartResult, PostgresError> {
        let tx = self.db.begin().await?;
        let Some(attachment) = lock_authorized_upload(&tx, attachment_id, uploader_user_id).await?
        else {
            tx.commit().await?;
            return Ok(StorePartResult::WriteLeaseLost);
        };
        let existing = part_by_number(&tx, attachment_id, part_number)
            .await?
            .ok_or_else(|| PostgresError::not_found("upload part reservation not found"))?;
        if existing.part.object_key != object_key {
            tx.commit().await?;
            return Ok(StorePartResult::WriteLeaseLost);
        }
        if existing.stored {
            tx.commit().await?;
            return Ok(StorePartResult::AlreadyRecorded(existing.part));
        }
        if authorize_active_upload(&attachment, upload_id, uploader_user_id, now_unix).is_err() {
            tx.commit().await?;
            return Ok(StorePartResult::WriteLeaseLost);
        }
        if existing.write_token.as_deref() != Some(write_token)
            || existing
                .write_expires_at_unix
                .is_none_or(|expires| expires <= now_unix)
        {
            tx.commit().await?;
            return Ok(StorePartResult::WriteLeaseLost);
        }
        let updated = tx
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_upload_parts
             SET state = 'Stored', stored_at_unix = $3,
                 write_token = NULL, write_expires_at_unix = NULL
             WHERE attachment_id = $1 AND part_number = $2 AND state = 'Pending'
               AND write_token = $4 AND write_expires_at_unix > $3",
                [
                    attachment_id.into(),
                    as_i32(part_number, "part number")?.into(),
                    as_i64(now_unix, "part stored time")?.into(),
                    write_token.into(),
                ],
            ))
            .await?;
        if updated.rows_affected() != 1 {
            tx.commit().await?;
            return Ok(StorePartResult::WriteLeaseLost);
        }
        tx.commit().await?;
        Ok(StorePartResult::Recorded)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "release_upload_part_write"))]
    pub async fn release_upload_part_write(
        &self,
        attachment_id: &str,
        part_number: u32,
        write_token: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_upload_parts SET write_expires_at_unix = $4
                 WHERE attachment_id = $1 AND part_number = $2 AND state = 'Pending'
                   AND write_token = $3 AND write_expires_at_unix > $4",
                [
                    attachment_id.into(),
                    as_i32(part_number, "part number")?.into(),
                    write_token.into(),
                    as_i64(now_unix, "part release time")?.into(),
                ],
            ))
            .await?;
        Ok(())
    }
}

struct UploadPartRow {
    part: RequestAttachmentStagedPart,
    stored: bool,
    write_token: Option<String>,
    write_expires_at_unix: Option<u64>,
}

async fn part_by_number<C>(
    conn: &C,
    attachment_id: &str,
    part_number: u32,
) -> Result<Option<UploadPartRow>, PostgresError>
where
    C: ConnectionTrait,
{
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM scope_request_media_upload_parts
             WHERE attachment_id = $1 AND part_number = $2 FOR UPDATE",
            [
                attachment_id.into(),
                as_i32(part_number, "part number")?.into(),
            ],
        ))
        .await?;
    row.map(part_from_row).transpose()
}

fn part_from_row(row: QueryResult) -> Result<UploadPartRow, PostgresError> {
    let state = row.try_get::<String>("", "state")?;
    Ok(UploadPartRow {
        part: RequestAttachmentStagedPart {
            receipt: RequestAttachmentPartReceipt {
                part_number: u32::try_from(row.try_get::<i32>("", "part_number")?)
                    .map_err(PostgresError::internal)?,
                size_bytes: u64::try_from(row.try_get::<i64>("", "plaintext_size_bytes")?)
                    .map_err(PostgresError::internal)?,
                sha256: row.try_get("", "sha256")?,
            },
            object_key: row.try_get("", "object_key")?,
        },
        stored: state == "Stored",
        write_token: row.try_get("", "write_token")?,
        write_expires_at_unix: row
            .try_get::<Option<i64>>("", "write_expires_at_unix")?
            .map(u64::try_from)
            .transpose()
            .map_err(PostgresError::internal)?,
    })
}

pub(super) async fn stored_parts<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Vec<RequestAttachmentStagedPart>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT * FROM scope_request_media_upload_parts
         WHERE attachment_id = $1 AND state = 'Stored' ORDER BY part_number",
        [attachment_id.into()],
    ))
    .await?
    .into_iter()
    .map(|row| part_from_row(row).map(|row| row.part))
    .collect()
}
