use super::{
    FinishRequestAttachmentUploadCommand, MediaStore, PrepareRequestAttachmentCommand,
    PreparedRequestAttachment,
    budget::{lock_media_budget, media_usage},
    locks::lock_attachment,
    persistence::{as_i64, attachment_by_id, enum_string, insert_manifest},
    upload_parts::stored_parts,
};
use crate::{
    db::{
        request_access::{ensure_user_exists, lock_request_repository, request_policy_for_user},
        request_media::access::cleanup_tombstone_exists,
    },
    error::{PostgresError, PostgresErrorKind},
};
use scope_domain::requests::attachments::{
    PrepareRequestAttachmentInput, RequestAttachment, RequestAttachmentLimits,
    RequestAttachmentState, RequestAttachmentStoredObject, RequestAttachmentTarget,
    RequestMediaObject, finish_attachment_upload, validate_prepare_attachment,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

impl MediaStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "prepare_request_attachment"))]
    pub async fn prepare_request_attachment(
        &self,
        command: PrepareRequestAttachmentCommand,
    ) -> Result<PreparedRequestAttachment, PostgresError> {
        let tx = self.db.begin().await?;
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
            let acknowledged_parts = stored_parts(&tx, &existing.id)
                .await?
                .into_iter()
                .map(|part| part.receipt)
                .collect();
            tx.commit().await?;
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
        let usage = media_usage(&tx, &repo.record.id, Some(&command.request_id), None).await?;
        let decision = validate_prepare_attachment(PrepareRequestAttachmentInput {
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
            request_source_bytes: usage.request_source_bytes,
            repository_reserved_bytes: usage.repository_bytes,
            now_unix: command.now_unix,
        })?;
        insert_prepared_attachment(
            &tx,
            &decision.attachment,
            decision.reserved_source_bytes,
            decision.reserved_derivative_bytes,
        )
        .await?;
        tx.commit().await?;
        Ok(PreparedRequestAttachment {
            upload_id: decision.attachment.upload_id.clone(),
            attachment: decision.attachment,
            acknowledged_parts: Vec::new(),
        })
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "finish_request_attachment_upload"))]
    pub async fn finish_request_attachment_upload(
        &self,
        command: FinishRequestAttachmentUploadCommand,
    ) -> Result<RequestAttachment, PostgresError> {
        let tx = self.db.begin().await?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        let policy = request_policy_for_user(&tx, &repo, &request, &command.actor_user_id).await?;
        let attachment = lock_attachment(&tx, &command.attachment_id).await?;
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
        )?;
        if attachment.state == RequestAttachmentState::Prepared {
            insert_manifest(
                &tx,
                &manifest_id,
                &attachment.id,
                None,
                &RequestMediaObject::from_parts(
                    attachment.declared_media_type.clone(),
                    attachment.sha256.clone(),
                    stored_parts,
                ),
                command.now_unix,
            )
            .await?;
            let unbound_expires_at_unix = command
                .now_unix
                .checked_add(RequestAttachmentLimits::default().unbound_attachment_ttl_seconds)
                .ok_or_else(|| PostgresError::internal_message("unbound expiry overflow"))?;
            tx.execute_raw(Statement::from_sql_and_values(
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
            .await?;
            tx.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_request_media_processing_jobs (
                    attachment_id, state, available_at_unix, created_at_unix, updated_at_unix, traceparent
                 ) VALUES ($1, 'Queued', $2, $2, $2, $3)
                 ON CONFLICT (attachment_id) DO NOTHING",
                [
                    attachment.id.clone().into(),
                    as_i64(command.now_unix, "processing enqueue time")?.into(),
                    crate::job_trace::QueuedTrace::current().into_column().into(),
                ],
            ))
            .await?;
        }
        tx.commit().await?;
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
    if let RequestAttachmentTarget::Discussion {
        discussion_id: Some(discussion_id),
    }
    | RequestAttachmentTarget::Reply { discussion_id } = target
    {
        ensure_discussion_belongs_to_request(conn, request_id, discussion_id).await?;
    }
    Ok(target.writable_with(&policy.permissions))
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
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT 1 AS present FROM scope_request_discussions WHERE id = $1 AND request_id = $2",
            [discussion_id.into(), request_id.into()],
        ))
        .await?
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
        .query_one_raw(Statement::from_sql_and_values(
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
        .await?
        .ok_or_else(|| PostgresError::internal_message("attachment target count missing"))?
        .try_get::<i64>("", "count")?;
    usize::try_from(count).map_err(PostgresError::internal)
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
        .query_one_raw(Statement::from_sql_and_values(
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
        .await?;
    match row {
        Some(row) => {
            let id = row.try_get::<String>("", "id")?;
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
    conn.execute_raw(Statement::from_sql_and_values(
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
    .await?;
    Ok(())
}

pub(super) async fn lock_authorized_upload<C>(
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
    if !writable {
        return Ok(None);
    }
    let attachment = lock_attachment(conn, attachment_id).await?;
    if attachment.request_id != request.id
        || attachment.uploader_user_id != uploader_user_id
        || cleanup_tombstone_exists(conn, attachment_id).await?
    {
        return Ok(None);
    }
    Ok(Some(attachment))
}

pub(super) fn authorize_active_upload(
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
