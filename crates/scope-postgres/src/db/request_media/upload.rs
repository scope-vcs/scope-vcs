use super::{
    FinishRequestAttachmentUploadCommand, MediaStore, PrepareRequestAttachmentCommand,
    PreparedRequestAttachment, ReserveUploadPartResult, StorePartResult,
    StoredRequestAttachmentPart, default_limits,
    persistence::{as_i32, as_i64, attachment_by_id, enum_string},
};
use crate::{
    db::{
        request_access::{ensure_user_exists, lock_request_repository, request_policy_for_user},
        request_media::access::cleanup_tombstone_exists,
    },
    error::{PostgresError, PostgresErrorKind},
};
use scope_domain::requests::attachments::{
    PrepareRequestAttachmentInput, RequestAttachment, RequestAttachmentPartReceipt,
    RequestAttachmentState, RequestAttachmentStoredObject, RequestAttachmentTarget,
    finish_attachment_upload, validate_prepare_attachment,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, TransactionTrait};

impl MediaStore {
    pub async fn prepare_request_attachment(
        &self,
        command: PrepareRequestAttachmentCommand,
        limits: scope_domain::requests::attachments::RequestAttachmentLimits,
    ) -> Result<PreparedRequestAttachment, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let policy = request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        let actor_can_write_target =
            target_is_writable(&tx, &command.request_id, &command.target, &policy).await?;

        if let Some(existing) = attachment_for_operation(
            &tx,
            &command.request_id,
            &command.actor_user_id,
            &command.operation_id,
        )
        .await?
        {
            if !actor_can_write_target {
                return Err(PostgresError::permission_denied(
                    "request attachment target write access required",
                ));
            }
            ensure_prepare_intent_matches(&existing, &command)?;
            if existing.state == RequestAttachmentState::Prepared
                && command.now_unix >= existing.upload_expires_at_unix
            {
                return Err(PostgresError::attachment_upload_expired(
                    "attachment upload expired; prepare a new upload operation",
                ));
            }
            if cleanup_tombstone_exists(&tx, &existing.id).await? {
                return Err(PostgresError::conflict(
                    "request attachment is being deleted",
                ));
            }
            let acknowledged_parts = stored_receipts(&tx, &existing.id).await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(PreparedRequestAttachment {
                upload_id: existing.upload_id.clone(),
                attachment: existing,
                acknowledged_parts,
            });
        }
        if attachment_by_id(&tx, &command.attachment_id)
            .await?
            .is_some()
        {
            return Err(PostgresError::conflict(
                "request attachment id is already in use",
            ));
        }
        lock_media_budget(&tx, &repo.record.id).await?;
        let target_attachment_count = count_target_attachments(
            &tx,
            &command.request_id,
            &command.actor_user_id,
            &command.target,
        )
        .await?;
        let request_source_bytes =
            reserved_source_usage(&tx, "request_id", &command.request_id).await?;
        let repository_reserved_bytes =
            reserved_total_usage(&tx, "repository_id", &repo.record.id).await?;
        let decision = validate_prepare_attachment(
            PrepareRequestAttachmentInput {
                attachment_id: command.attachment_id,
                repository_id: repo.record.id.clone(),
                request_id: command.request_id,
                uploader_user_id: command.actor_user_id,
                upload_id: command.upload_id,
                operation_id: command.operation_id,
                target: command.target,
                filename: command.filename,
                declared_media_type: command.declared_media_type,
                size_bytes: command.size_bytes,
                sha256: command.sha256,
                actor_can_write_target,
                request_is_open: !request.is_terminal(),
                target_attachment_count,
                request_source_bytes,
                repository_reserved_bytes,
                now_unix: command.now_unix,
            },
            limits,
        )?;
        insert_prepared_attachment(
            &tx,
            &decision.attachment,
            decision.reserved_source_bytes,
            decision.reserved_derivative_bytes,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(PreparedRequestAttachment {
            upload_id: decision.attachment.upload_id.clone(),
            attachment: decision.attachment,
            acknowledged_parts: Vec::new(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn reserve_upload_part(
        &self,
        attachment_id: &str,
        upload_id: &str,
        uploader_user_id: &str,
        part: StoredRequestAttachmentPart,
        write_token: &str,
        now_unix: u64,
        write_expires_at_unix: u64,
    ) -> Result<ReserveUploadPartResult, PostgresError> {
        validate_write_lease(write_token, now_unix, write_expires_at_unix)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let attachment = lock_authorized_upload(&tx, attachment_id, uploader_user_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request attachment upload not found"))?;
        authorize_active_upload(&attachment, upload_id, uploader_user_id, now_unix)?;
        scope_domain::requests::attachments::validate_attachment_part(
            &attachment,
            &part.receipt,
            default_limits(),
        )?;
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
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(ReserveUploadPartResult::Stored(existing.part));
            }
            if existing
                .write_expires_at_unix
                .is_some_and(|expires| expires > now_unix)
            {
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(ReserveUploadPartResult::Busy);
            }
            if existing.part.object_key == part.object_key {
                return Err(PostgresError::conflict(
                    "expired upload part retries require a fresh object key",
                ));
            }
            tx.execute(Statement::from_sql_and_values(
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
            .await
            .map_err(PostgresError::internal)?;
            tx.execute(Statement::from_sql_and_values(
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
            .await
            .map_err(PostgresError::internal)?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(ReserveUploadPartResult::Write(part));
        }
        tx.execute(Statement::from_sql_and_values(
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
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(ReserveUploadPartResult::Write(part))
    }

    #[allow(clippy::too_many_arguments)]
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
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(attachment) = lock_authorized_upload(&tx, attachment_id, uploader_user_id).await?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(StorePartResult::WriteLeaseLost);
        };
        let existing = part_by_number(&tx, attachment_id, part_number)
            .await?
            .ok_or_else(|| PostgresError::not_found("upload part reservation not found"))?;
        if existing.part.object_key != object_key {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(StorePartResult::WriteLeaseLost);
        }
        if existing.stored {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(StorePartResult::AlreadyRecorded(existing.part));
        }
        if authorize_active_upload(&attachment, upload_id, uploader_user_id, now_unix).is_err() {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(StorePartResult::WriteLeaseLost);
        }
        if existing.write_token.as_deref() != Some(write_token)
            || existing
                .write_expires_at_unix
                .is_none_or(|expires| expires <= now_unix)
        {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(StorePartResult::WriteLeaseLost);
        }
        let updated = tx
            .execute(Statement::from_sql_and_values(
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
            .await
            .map_err(PostgresError::internal)?;
        if updated.rows_affected() != 1 {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(StorePartResult::WriteLeaseLost);
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(StorePartResult::Recorded)
    }

    pub async fn finish_request_attachment_upload(
        &self,
        command: FinishRequestAttachmentUploadCommand,
    ) -> Result<RequestAttachment, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        let policy = request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        let attachment = lock_upload_attachment(&tx, &command.attachment_id).await?;
        if attachment.request_id != request.id
            || cleanup_tombstone_exists(&tx, &attachment.id).await?
        {
            return Err(PostgresError::not_found("request attachment not found"));
        }
        if !target_is_writable(&tx, &request.id, &attachment.target, &policy).await? {
            return Err(PostgresError::permission_denied(
                "request attachment target write access required",
            ));
        }
        let stored_parts = stored_parts(&tx, &attachment.id).await?;
        let receipts = stored_parts
            .iter()
            .map(|part| part.receipt.clone())
            .collect::<Vec<_>>();
        if receipts != command.parts {
            return Err(PostgresError::conflict(
                "finish receipts do not match stored upload parts",
            ));
        }
        let manifest_id = format!("{}:original", attachment.id);
        let original = RequestAttachmentStoredObject {
            object_key: manifest_id.clone(),
            size_bytes: attachment.size_bytes,
            sha256: attachment.sha256.clone(),
        };
        let next = finish_attachment_upload(
            &attachment,
            &command.actor_user_id,
            &command.upload_id,
            &receipts,
            original,
            command.now_unix,
            default_limits(),
        )?;
        if attachment.state == RequestAttachmentState::Prepared {
            insert_original_manifest(
                &tx,
                &attachment,
                &manifest_id,
                &stored_parts,
                command.now_unix,
            )
            .await?;
            let unbound_expires_at_unix = command
                .now_unix
                .checked_add(default_limits().unbound_attachment_ttl_seconds)
                .ok_or_else(|| PostgresError::internal_message("unbound expiry overflow"))?;
            tx.execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_attachments
                 SET state = 'Uploaded', original_manifest_id = $2,
                     updated_at_unix = $3, unbound_expires_at_unix = $4
                 WHERE id = $1",
                [
                    attachment.id.clone().into(),
                    manifest_id.into(),
                    as_i64(command.now_unix, "upload finish time")?.into(),
                    as_i64(unbound_expires_at_unix, "unbound expiry")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
            tx.execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_request_media_processing_jobs (
                    attachment_id, state, available_at_unix, created_at_unix, updated_at_unix
                 ) VALUES ($1, 'Queued', $2, $2, $2)
                 ON CONFLICT (attachment_id) DO NOTHING",
                [
                    attachment.id.clone().into(),
                    as_i64(command.now_unix, "processing enqueue time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(next)
    }
}

async fn target_is_writable<C>(
    conn: &C,
    request_id: &str,
    target: &RequestAttachmentTarget,
    policy: &scope_domain::requests::RequestPolicyDecision,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    match target {
        RequestAttachmentTarget::Description => Ok(policy.permissions.can_edit_identity),
        RequestAttachmentTarget::Discussion { discussion_id } => {
            if let Some(discussion_id) = discussion_id {
                ensure_discussion_belongs_to_request(conn, request_id, discussion_id).await?;
            }
            Ok(policy.permissions.can_open_discussion)
        }
        RequestAttachmentTarget::Reply { discussion_id } => {
            ensure_discussion_belongs_to_request(conn, request_id, discussion_id).await?;
            Ok(policy.permissions.can_reply_to_discussion)
        }
    }
}

async fn ensure_discussion_belongs_to_request<C>(
    conn: &C,
    request_id: &str,
    discussion_id: &str,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let exists = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT 1 AS present FROM scope_request_discussions WHERE id = $1 AND request_id = $2",
            [discussion_id.into(), request_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(PostgresError::not_found("request discussion not found"))
    }
}

async fn count_target_attachments<C>(
    conn: &C,
    request_id: &str,
    uploader_user_id: &str,
    target: &RequestAttachmentTarget,
) -> Result<usize, PostgresError>
where
    C: ConnectionTrait,
{
    let target_json = serde_json::to_value(target).map_err(PostgresError::internal)?;
    let count = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT COUNT(*)::bigint AS count
             FROM scope_request_media_attachments attachment
             WHERE attachment.request_id = $1
               AND attachment.uploader_user_id = $2
               AND attachment.target_json = $3
               AND NOT EXISTS (
                    SELECT 1 FROM scope_request_media_bindings binding
                    WHERE binding.attachment_id = attachment.id
               )
               AND NOT EXISTS (
                    SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                    WHERE cleanup.attachment_id = attachment.id
               )",
            [
                request_id.into(),
                uploader_user_id.into(),
                target_json.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("attachment target count missing"))?
        .try_get::<i64>("", "count")
        .map_err(PostgresError::internal)?;
    usize::try_from(count).map_err(PostgresError::internal)
}

async fn reserved_source_usage<C>(conn: &C, column: &str, id: &str) -> Result<u64, PostgresError>
where
    C: ConnectionTrait,
{
    let sql = format!(
        "SELECT COALESCE(SUM(attachment.reserved_source_bytes), 0)::bigint AS bytes
         FROM scope_request_media_attachments attachment
         WHERE attachment.{column} = $1
           AND NOT EXISTS (
                SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                WHERE cleanup.attachment_id = attachment.id AND cleanup.state = 'Completed'
           )"
    );
    let bytes = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            [id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("attachment usage missing"))?
        .try_get::<i64>("", "bytes")
        .map_err(PostgresError::internal)?;
    u64::try_from(bytes).map_err(PostgresError::internal)
}

async fn reserved_total_usage<C>(conn: &C, column: &str, id: &str) -> Result<u64, PostgresError>
where
    C: ConnectionTrait,
{
    let sql = format!(
        "SELECT COALESCE(SUM(
            attachment.reserved_source_bytes
            + COALESCE(attachment.actual_derivative_bytes, attachment.reserved_derivative_bytes)
         ), 0)::bigint AS bytes
         FROM scope_request_media_attachments attachment
         WHERE attachment.{column} = $1
           AND NOT EXISTS (
                SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                WHERE cleanup.attachment_id = attachment.id AND cleanup.state = 'Completed'
           )"
    );
    let bytes = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            [id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("attachment usage missing"))?
        .try_get::<i64>("", "bytes")
        .map_err(PostgresError::internal)?;
    u64::try_from(bytes).map_err(PostgresError::internal)
}

pub(super) async fn lock_media_budget<C>(conn: &C, repository_id: &str) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT pg_advisory_xact_lock(hashtextextended('scope:request-media-budget:' || $1, 0))",
        [repository_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

async fn attachment_for_operation<C>(
    conn: &C,
    request_id: &str,
    uploader_user_id: &str,
    operation_id: &str,
) -> Result<Option<RequestAttachment>, PostgresError>
where
    C: ConnectionTrait,
{
    let row = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT id FROM scope_request_media_attachments
             WHERE request_id = $1 AND uploader_user_id = $2 AND operation_id = $3
             FOR UPDATE",
            [
                request_id.into(),
                uploader_user_id.into(),
                operation_id.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    match row {
        Some(row) => {
            let id = row
                .try_get::<String>("", "id")
                .map_err(PostgresError::internal)?;
            attachment_by_id(conn, &id).await
        }
        None => Ok(None),
    }
}

fn ensure_prepare_intent_matches(
    attachment: &RequestAttachment,
    command: &PrepareRequestAttachmentCommand,
) -> Result<(), PostgresError> {
    let matches = attachment.target == command.target
        && attachment.filename == command.filename
        && attachment
            .declared_media_type
            .eq_ignore_ascii_case(&command.declared_media_type)
        && attachment.size_bytes == command.size_bytes
        && attachment.sha256.eq_ignore_ascii_case(&command.sha256);
    if matches {
        Ok(())
    } else {
        Err(PostgresError::conflict(
            "request attachment operation id was reused with different input",
        ))
    }
}

async fn insert_prepared_attachment<C>(
    conn: &C,
    attachment: &RequestAttachment,
    reserved_source_bytes: u64,
    reserved_derivative_bytes: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_request_media_attachments (
            id, repository_id, request_id, uploader_user_id, upload_id, operation_id,
            target_json, filename, declared_media_type, kind, size_bytes, sha256, state,
            reserved_source_bytes, reserved_derivative_bytes,
            created_at_unix, updated_at_unix, upload_expires_at_unix
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$16,$17)",
        [
            attachment.id.clone().into(),
            attachment.repository_id.clone().into(),
            attachment.request_id.clone().into(),
            attachment.uploader_user_id.clone().into(),
            attachment.upload_id.clone().into(),
            attachment.operation_id.clone().into(),
            serde_json::to_value(&attachment.target)
                .map_err(PostgresError::internal)?
                .into(),
            attachment.filename.clone().into(),
            attachment.declared_media_type.clone().into(),
            enum_string(attachment.kind)?.into(),
            as_i64(attachment.size_bytes, "attachment size")?.into(),
            attachment.sha256.clone().into(),
            enum_string(attachment.state)?.into(),
            as_i64(reserved_source_bytes, "source reservation")?.into(),
            as_i64(reserved_derivative_bytes, "derivative reservation")?.into(),
            as_i64(attachment.created_at_unix, "attachment creation time")?.into(),
            as_i64(attachment.upload_expires_at_unix, "upload expiry")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

async fn lock_upload_attachment<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<RequestAttachment, PostgresError>
where
    C: ConnectionTrait,
{
    let row = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT id FROM scope_request_media_attachments WHERE id = $1 FOR UPDATE",
            [attachment_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found("request attachment not found"))?;
    let id = row
        .try_get::<String>("", "id")
        .map_err(PostgresError::internal)?;
    attachment_by_id(conn, &id)
        .await?
        .ok_or_else(|| PostgresError::not_found("request attachment not found"))
}

async fn lock_authorized_upload<C>(
    conn: &C,
    attachment_id: &str,
    uploader_user_id: &str,
) -> Result<Option<RequestAttachment>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(observed) = attachment_by_id(conn, attachment_id).await? else {
        return Ok(None);
    };
    if observed.uploader_user_id != uploader_user_id {
        return Ok(None);
    }
    let (repo, request) =
        match lock_request_repository(conn, &observed.request_id, uploader_user_id).await {
            Ok(value) => value,
            Err(error)
                if matches!(
                    error.kind,
                    PostgresErrorKind::NotFound
                        | PostgresErrorKind::PermissionDenied
                        | PostgresErrorKind::Unauthenticated
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
    let policy = request_policy_for_user(conn, &repo, &request, uploader_user_id).await?;
    let writable = match target_is_writable(conn, &request.id, &observed.target, &policy).await {
        Ok(value) => value,
        Err(error) if error.kind == PostgresErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !writable || cleanup_tombstone_exists(conn, attachment_id).await? {
        return Ok(None);
    }
    let attachment = lock_upload_attachment(conn, attachment_id).await?;
    if attachment.request_id != request.id
        || attachment.uploader_user_id != uploader_user_id
        || cleanup_tombstone_exists(conn, attachment_id).await?
    {
        return Ok(None);
    }
    Ok(Some(attachment))
}

fn authorize_active_upload(
    attachment: &RequestAttachment,
    upload_id: &str,
    uploader_user_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    if attachment.upload_id != upload_id || attachment.uploader_user_id != uploader_user_id {
        return Err(PostgresError::not_found(
            "request attachment upload not found",
        ));
    }
    if attachment.state != RequestAttachmentState::Prepared {
        return Err(PostgresError::conflict(
            "request attachment upload is sealed",
        ));
    }
    if now_unix >= attachment.upload_expires_at_unix {
        return Err(PostgresError::conflict("request attachment upload expired"));
    }
    Ok(())
}

fn validate_write_lease(
    write_token: &str,
    now_unix: u64,
    write_expires_at_unix: u64,
) -> Result<(), PostgresError> {
    if write_token.trim().is_empty() || write_expires_at_unix <= now_unix {
        return Err(PostgresError::invalid_input(
            "upload part write lease must have a token and future expiry",
        ));
    }
    Ok(())
}

struct UploadPartRow {
    part: StoredRequestAttachmentPart,
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
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM scope_request_media_upload_parts
             WHERE attachment_id = $1 AND part_number = $2 FOR UPDATE",
            [
                attachment_id.into(),
                as_i32(part_number, "part number")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    row.map(part_from_row).transpose()
}

fn part_from_row(row: QueryResult) -> Result<UploadPartRow, PostgresError> {
    let state = row
        .try_get::<String>("", "state")
        .map_err(PostgresError::internal)?;
    Ok(UploadPartRow {
        part: StoredRequestAttachmentPart {
            receipt: RequestAttachmentPartReceipt {
                part_number: u32::try_from(
                    row.try_get::<i32>("", "part_number")
                        .map_err(PostgresError::internal)?,
                )
                .map_err(PostgresError::internal)?,
                size_bytes: u64::try_from(
                    row.try_get::<i64>("", "plaintext_size_bytes")
                        .map_err(PostgresError::internal)?,
                )
                .map_err(PostgresError::internal)?,
                sha256: row.try_get("", "sha256").map_err(PostgresError::internal)?,
            },
            object_key: row
                .try_get("", "object_key")
                .map_err(PostgresError::internal)?,
        },
        stored: state == "Stored",
        write_token: row
            .try_get("", "write_token")
            .map_err(PostgresError::internal)?,
        write_expires_at_unix: row
            .try_get::<Option<i64>>("", "write_expires_at_unix")
            .map_err(PostgresError::internal)?
            .map(u64::try_from)
            .transpose()
            .map_err(PostgresError::internal)?,
    })
}

async fn stored_parts<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Vec<StoredRequestAttachmentPart>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_all(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT * FROM scope_request_media_upload_parts
         WHERE attachment_id = $1 AND state = 'Stored' ORDER BY part_number",
        [attachment_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)?
    .into_iter()
    .map(|row| part_from_row(row).map(|row| row.part))
    .collect()
}

async fn stored_receipts<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Vec<RequestAttachmentPartReceipt>, PostgresError>
where
    C: ConnectionTrait,
{
    Ok(stored_parts(conn, attachment_id)
        .await?
        .into_iter()
        .map(|part| part.receipt)
        .collect())
}

async fn insert_original_manifest<C>(
    conn: &C,
    attachment: &RequestAttachment,
    manifest_id: &str,
    parts: &[StoredRequestAttachmentPart],
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_request_media_manifests (
            id, attachment_id, derivative_id, media_type, size_bytes, sha256, completed_at_unix
         ) VALUES ($1, $2, NULL, $3, $4, $5, $6)",
        [
            manifest_id.into(),
            attachment.id.clone().into(),
            attachment.declared_media_type.clone().into(),
            as_i64(attachment.size_bytes, "manifest size")?.into(),
            attachment.sha256.clone().into(),
            as_i64(now_unix, "manifest completion time")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    let mut offset = 0_u64;
    for part in parts {
        conn.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_request_media_manifest_chunks (
                manifest_id, chunk_index, object_key, plaintext_offset,
                plaintext_size_bytes, sha256
             ) VALUES ($1, $2, $3, $4, $5, $6)",
            [
                manifest_id.into(),
                as_i32(part.receipt.part_number, "chunk index")?.into(),
                part.object_key.clone().into(),
                as_i64(offset, "chunk offset")?.into(),
                as_i64(part.receipt.size_bytes, "chunk size")?.into(),
                part.receipt.sha256.clone().into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        offset = offset
            .checked_add(part.receipt.size_bytes)
            .ok_or_else(|| PostgresError::internal_message("manifest offset overflow"))?;
    }
    Ok(())
}
