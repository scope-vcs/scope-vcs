use super::{
    CompleteRequestAttachmentProcessingCommand, FailRequestAttachmentProcessingCommand,
    MediaLeaseMutation, MediaStore, RequestMediaManifest, ValidateRequestAttachmentSourceCommand,
    access::cleanup_tombstone_exists,
    persistence::{as_i32, as_i64, attachment_by_id, manifest_by_id},
    processing_support::{
        adopt_manifest_keys, complete_job, ensure_derivative_budget, ensure_manifest_keys_reserved,
        insert_derivative, lock_attachment, lock_lease_and_attachment,
        lock_lease_attachment_and_budget, lock_processing_job, save_attachment_processing_state,
        save_completed_attachment, save_validated_source, source_metadata,
        validate_completed_derivatives, validate_new_lease, validate_source_identity,
    },
};
use crate::{
    db::{
        locks::acquire_shared_repository_lock,
        request_access::{lock_request_repository, request_policy_for_user},
    },
    error::PostgresError,
};
use scope_domain::requests::attachments::{
    RequestAttachment, RequestAttachmentProcessingLease, RequestAttachmentState,
    mark_processing_source_validated, retry_attachment_processing, transition_attachment,
    validate_processing_completion, validate_processing_failure,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

impl MediaStore {
    pub async fn claim_processing_job(
        &self,
        lease_token: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<Option<RequestAttachmentProcessingLease>, PostgresError> {
        validate_new_lease(lease_token, now_unix, lease_expires_at_unix)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(candidate) = tx
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT job.attachment_id, attachment.repository_id
                 FROM scope_request_media_processing_jobs job
                 JOIN scope_request_media_attachments attachment ON attachment.id = job.attachment_id
                 JOIN scope_repositories repository ON repository.id = attachment.repository_id
                 WHERE (
                    (job.state = 'Queued' AND job.available_at_unix <= $1)
                    OR (job.state = 'Leased' AND job.lease_expires_at_unix <= $1)
                 )
                   AND attachment.state IN ('Uploaded', 'Processing', 'Failed')
                   AND NOT EXISTS (
                        SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                        WHERE cleanup.attachment_id = attachment.id
                   )
                 ORDER BY job.available_at_unix, job.created_at_unix, job.attachment_id
                 LIMIT 1",
                [as_i64(now_unix, "processing claim time")?.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let attachment_id = candidate
            .try_get::<String>("", "attachment_id")
            .map_err(PostgresError::internal)?;
        let repository_id = candidate
            .try_get::<String>("", "repository_id")
            .map_err(PostgresError::internal)?;
        acquire_shared_repository_lock(&tx, &repository_id).await?;
        let Some(job) = tx
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT job.*
                 FROM scope_request_media_processing_jobs job
                 JOIN scope_request_media_attachments attachment ON attachment.id = job.attachment_id
                 JOIN scope_repositories repository ON repository.id = attachment.repository_id
                 WHERE job.attachment_id = $2
                   AND ((job.state = 'Queued' AND job.available_at_unix <= $1)
                        OR (job.state = 'Leased' AND job.lease_expires_at_unix <= $1))
                   AND attachment.state IN ('Uploaded', 'Processing', 'Failed')
                   AND NOT EXISTS (
                        SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                        WHERE cleanup.attachment_id = attachment.id
                   )
                 FOR UPDATE OF job SKIP LOCKED",
                [
                    as_i64(now_unix, "processing claim time")?.into(),
                    attachment_id.clone().into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let old_generation = u64::try_from(
            job.try_get::<i64>("", "lease_generation")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?;
        let old_state = job
            .try_get::<String>("", "state")
            .map_err(PostgresError::internal)?;
        let mut attachment = lock_attachment(&tx, &attachment_id).await?;
        if old_state == "Leased" {
            tx.execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_processing_objects
                 SET state = 'Orphaned', updated_at_unix = $3
                 WHERE attachment_id = $1 AND lease_generation = $2 AND state = 'Pending'",
                [
                    attachment_id.clone().into(),
                    as_i64(old_generation, "old lease generation")?.into(),
                    as_i64(now_unix, "orphan time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        }
        if attachment.state == RequestAttachmentState::Uploaded {
            attachment = transition_attachment(
                &attachment,
                RequestAttachmentState::Processing,
                None,
                now_unix,
            )?;
            save_attachment_processing_state(&tx, &attachment).await?;
        } else if attachment.state == RequestAttachmentState::Failed {
            attachment = retry_attachment_processing(&attachment, true, now_unix)?;
            save_attachment_processing_state(&tx, &attachment).await?;
        }
        let generation = old_generation.checked_add(1).ok_or_else(|| {
            PostgresError::internal_message("processing lease generation overflow")
        })?;
        let attempt = u32::try_from(
            job.try_get::<i32>("", "attempt")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?
        .checked_add(1)
        .ok_or_else(|| PostgresError::internal_message("processing attempt overflow"))?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_request_media_processing_jobs
             SET state = 'Leased', lease_token = $2, lease_generation = $3,
                 lease_expires_at_unix = $4, attempt = $5, updated_at_unix = $6
             WHERE attachment_id = $1",
            [
                attachment_id.clone().into(),
                lease_token.into(),
                as_i64(generation, "processing lease generation")?.into(),
                as_i64(lease_expires_at_unix, "processing lease expiry")?.into(),
                as_i32(attempt, "processing attempt")?.into(),
                as_i64(now_unix, "processing claim time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        notify_attachment_change(&tx, &attachment).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(RequestAttachmentProcessingLease {
            attachment_id,
            repository_id: attachment.repository_id,
            request_id: attachment.request_id,
            lease_token: lease_token.to_string(),
            lease_generation: generation,
            attempt,
            lease_expires_at_unix,
        }))
    }

    pub async fn renew_processing_lease(
        &self,
        attachment_id: &str,
        lease_token: &str,
        lease_generation: u64,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<bool, PostgresError> {
        validate_new_lease(lease_token, now_unix, lease_expires_at_unix)?;
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_processing_jobs job
                 SET lease_expires_at_unix = $5, updated_at_unix = $4
                 WHERE job.attachment_id = $1 AND job.state = 'Leased'
                   AND job.lease_token = $2 AND job.lease_generation = $3
                   AND job.lease_expires_at_unix > $4
                   AND NOT EXISTS (
                        SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                        WHERE cleanup.attachment_id = job.attachment_id
                   )",
                [
                    attachment_id.into(),
                    lease_token.into(),
                    as_i64(lease_generation, "processing lease generation")?.into(),
                    as_i64(now_unix, "processing heartbeat time")?.into(),
                    as_i64(lease_expires_at_unix, "processing lease expiry")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn processing_source_manifest(
        &self,
        lease: &RequestAttachmentProcessingLease,
        now_unix: u64,
    ) -> Result<Option<RequestMediaManifest>, PostgresError> {
        let valid = self
            .db
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT attachment.original_manifest_id
                 FROM scope_request_media_processing_jobs job
                 JOIN scope_request_media_attachments attachment ON attachment.id = job.attachment_id
                 WHERE job.attachment_id = $1 AND job.state = 'Leased'
                   AND job.lease_token = $2 AND job.lease_generation = $3
                   AND job.lease_expires_at_unix > $4
                   AND NOT EXISTS (
                        SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                        WHERE cleanup.attachment_id = job.attachment_id
                   )",
                [
                    lease.attachment_id.clone().into(),
                    lease.lease_token.clone().into(),
                    as_i64(lease.lease_generation, "processing lease generation")?.into(),
                    as_i64(now_unix, "processing source read time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        let Some(row) = valid else {
            return Ok(None);
        };
        let Some(manifest_id) = row
            .try_get::<Option<String>>("", "original_manifest_id")
            .map_err(PostgresError::internal)?
        else {
            return Ok(None);
        };
        manifest_by_id(self.db.as_ref(), &manifest_id).await
    }

    pub async fn reserve_processing_object_key(
        &self,
        attachment_id: &str,
        lease_token: &str,
        lease_generation: u64,
        object_key: &str,
        now_unix: u64,
    ) -> Result<MediaLeaseMutation<()>, PostgresError> {
        if object_key.trim().is_empty() {
            return Err(PostgresError::invalid_input("media object key is required"));
        }
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        if lock_lease_and_attachment(&tx, attachment_id, lease_token, lease_generation, now_unix)
            .await?
            .is_none()
        {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(MediaLeaseMutation::LeaseLost);
        }
        if let Some(row) = tx
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT attachment_id, lease_generation, state
                 FROM scope_request_media_processing_objects WHERE object_key = $1 FOR UPDATE",
                [object_key.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
        {
            let same = row
                .try_get::<String>("", "attachment_id")
                .map_err(PostgresError::internal)?
                == attachment_id
                && u64::try_from(
                    row.try_get::<i64>("", "lease_generation")
                        .map_err(PostgresError::internal)?,
                )
                .map_err(PostgresError::internal)?
                    == lease_generation
                && row
                    .try_get::<String>("", "state")
                    .map_err(PostgresError::internal)?
                    == "Pending";
            if !same {
                return Err(PostgresError::conflict(
                    "media object key is already reserved by another processing lease",
                ));
            }
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(MediaLeaseMutation::Applied(()));
        }
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_request_media_processing_objects (
                object_key, attachment_id, lease_generation, state,
                created_at_unix, updated_at_unix
             ) VALUES ($1, $2, $3, 'Pending', $4, $4)",
            [
                object_key.into(),
                attachment_id.into(),
                as_i64(lease_generation, "processing lease generation")?.into(),
                as_i64(now_unix, "processing object reservation time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(MediaLeaseMutation::Applied(()))
    }

    pub async fn mark_processing_source_validated(
        &self,
        command: ValidateRequestAttachmentSourceCommand,
    ) -> Result<MediaLeaseMutation<RequestAttachment>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some((lease, attachment)) = lock_lease_and_attachment(
            &tx,
            &command.attachment_id,
            &command.lease_token,
            command.lease_generation,
            command.now_unix,
        )
        .await?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(MediaLeaseMutation::LeaseLost);
        };
        validate_source_identity(&attachment, &command.source)?;
        let (image, video) = source_metadata(attachment.kind, &command.source);
        let next = mark_processing_source_validated(
            &attachment,
            &lease,
            &command.lease_token,
            command.lease_generation,
            command.source.detected_media_type,
            image,
            video,
            command.now_unix,
        )?;
        save_validated_source(&tx, &next).await?;
        notify_attachment_change(&tx, &next).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(MediaLeaseMutation::Applied(next))
    }

    pub async fn complete_processing_job(
        &self,
        command: CompleteRequestAttachmentProcessingCommand,
    ) -> Result<MediaLeaseMutation<RequestAttachment>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some((lease, attachment)) = lock_lease_attachment_and_budget(
            &tx,
            &command.attachment_id,
            &command.lease_token,
            command.lease_generation,
            command.now_unix,
        )
        .await?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(MediaLeaseMutation::LeaseLost);
        };
        validate_source_identity(&attachment, &command.source)?;
        validate_completed_derivatives(&command.derivatives)?;
        for derivative in &command.derivatives {
            ensure_manifest_keys_reserved(
                &tx,
                &attachment.id,
                command.lease_generation,
                &derivative.manifest,
            )
            .await?;
        }
        let derivative_bytes = command.derivatives.iter().try_fold(0_u64, |total, value| {
            total
                .checked_add(value.manifest.size_bytes)
                .ok_or_else(|| PostgresError::invalid_input("derivative byte count overflow"))
        })?;
        ensure_derivative_budget(&tx, &attachment, derivative_bytes).await?;
        let (image, video) = source_metadata(attachment.kind, &command.source);
        let derivatives = command
            .derivatives
            .iter()
            .map(|value| value.derivative.clone())
            .collect::<Vec<_>>();
        let next = validate_processing_completion(
            &attachment,
            &lease,
            &command.lease_token,
            command.lease_generation,
            command.source.detected_media_type,
            image,
            video,
            derivatives,
            command.now_unix,
        )?;
        for derivative in &command.derivatives {
            insert_derivative(&tx, &attachment.id, derivative, command.now_unix).await?;
            adopt_manifest_keys(
                &tx,
                &attachment.id,
                command.lease_generation,
                &derivative.manifest,
                command.now_unix,
            )
            .await?;
        }
        save_completed_attachment(&tx, &next, derivative_bytes).await?;
        complete_job(&tx, &attachment.id, command.now_unix).await?;
        notify_attachment_change(&tx, &next).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(MediaLeaseMutation::Applied(next))
    }

    pub async fn fail_processing_job(
        &self,
        command: FailRequestAttachmentProcessingCommand,
    ) -> Result<MediaLeaseMutation<RequestAttachment>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some((lease, attachment)) = lock_lease_and_attachment(
            &tx,
            &command.attachment_id,
            &command.lease_token,
            command.lease_generation,
            command.now_unix,
        )
        .await?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(MediaLeaseMutation::LeaseLost);
        };
        let next = validate_processing_failure(
            &attachment,
            &lease,
            &command.lease_token,
            command.lease_generation,
            command.failure,
            attachment.original_validated_at_unix.is_some(),
            command.now_unix,
        )?;
        save_attachment_processing_state(&tx, &next).await?;
        let (state, available_at) = if next
            .failure
            .as_ref()
            .is_some_and(|failure| failure.retryable)
        {
            match command.retry_at_unix {
                Some(retry_at) if retry_at >= command.now_unix => ("Queued", retry_at),
                Some(_) => {
                    return Err(PostgresError::invalid_input(
                        "processing retry time cannot be in the past",
                    ));
                }
                None => ("Completed", command.now_unix),
            }
        } else {
            ("Completed", command.now_unix)
        };
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_request_media_processing_jobs
             SET state = $2, available_at_unix = $3, lease_token = NULL,
                 lease_expires_at_unix = NULL, updated_at_unix = $4
             WHERE attachment_id = $1",
            [
                attachment.id.clone().into(),
                state.into(),
                as_i64(available_at, "processing retry time")?.into(),
                as_i64(command.now_unix, "processing failure time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_request_media_processing_objects
             SET state = 'Orphaned', updated_at_unix = $3
             WHERE attachment_id = $1 AND lease_generation = $2 AND state = 'Pending'",
            [
                attachment.id.clone().into(),
                as_i64(command.lease_generation, "processing lease generation")?.into(),
                as_i64(command.now_unix, "processing failure time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        notify_attachment_change(&tx, &next).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(MediaLeaseMutation::Applied(next))
    }

    pub async fn retry_request_attachment_processing(
        &self,
        request_id: &str,
        attachment_id: &str,
        actor_user_id: &str,
        operation_id: &str,
        now_unix: u64,
    ) -> Result<RequestAttachment, PostgresError> {
        if operation_id.trim().is_empty() {
            return Err(PostgresError::invalid_input(
                "retry operation id is required",
            ));
        }
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) = lock_request_repository(&tx, request_id, actor_user_id).await?;
        let Some(observed) = attachment_by_id(&tx, attachment_id).await? else {
            return Err(PostgresError::not_found("request attachment not found"));
        };
        if observed.request_id != request.id || observed.repository_id != repo.record.id {
            return Err(PostgresError::not_found("request attachment not found"));
        }
        lock_processing_job(&tx, attachment_id).await?;
        let attachment = lock_attachment(&tx, attachment_id).await?;
        if attachment.request_id != request.id
            || cleanup_tombstone_exists(&tx, attachment_id).await?
        {
            return Err(PostgresError::not_found("request attachment not found"));
        }
        let existing_operation = tx
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT 1 AS present FROM scope_request_media_retry_operations
                 WHERE attachment_id = $1 AND operation_id = $2",
                [attachment_id.into(), operation_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .is_some();
        if existing_operation {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(attachment);
        }
        let policy = request_policy_for_user(&tx, &repo, &request, actor_user_id).await?;
        let actor_can_write = match &attachment.target {
            scope_domain::requests::attachments::RequestAttachmentTarget::Description => {
                policy.permissions.can_edit_identity
            }
            scope_domain::requests::attachments::RequestAttachmentTarget::Discussion { .. } => {
                policy.permissions.can_open_discussion
            }
            scope_domain::requests::attachments::RequestAttachmentTarget::Reply { .. } => {
                policy.permissions.can_reply_to_discussion
            }
        };
        let next = retry_attachment_processing(&attachment, actor_can_write, now_unix)?;
        save_attachment_processing_state(&tx, &next).await?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_request_media_retry_operations (
                attachment_id, operation_id, created_at_unix
             ) VALUES ($1, $2, $3)",
            [
                attachment_id.into(),
                operation_id.into(),
                as_i64(now_unix, "retry operation time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_request_media_processing_jobs
             SET state = 'Queued', available_at_unix = $2, lease_token = NULL,
                 lease_expires_at_unix = NULL, updated_at_unix = $2
             WHERE attachment_id = $1",
            [
                attachment_id.into(),
                as_i64(now_unix, "processing retry time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        notify_attachment_change(&tx, &next).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(next)
    }
}

pub(super) async fn notify_attachment_change<C>(
    conn: &C,
    attachment: &RequestAttachment,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let Some(row) = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT repository.incarnation_id, request.audience
             FROM scope_repositories repository
             JOIN scope_requests request ON request.repo_id = repository.id
             WHERE repository.id = $1 AND request.id = $2",
            [
                attachment.repository_id.clone().into(),
                attachment.request_id.clone().into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(());
    };
    let incarnation_id = row
        .try_get::<String>("", "incarnation_id")
        .map_err(PostgresError::internal)?;
    let audience = row
        .try_get::<String>("", "audience")
        .map_err(PostgresError::internal)?;
    let payload = serde_json::json!({
        "event": {
            "repo_id": attachment.repository_id,
            "incarnation_id": incarnation_id,
            "version": 0,
            "kind": {
                "RequestAttachmentChanged": {
                    "request_id": attachment.request_id,
                    "attachment_id": attachment.id,
                    "audience": audience,
                }
            }
        },
        "origin_id": "scope-postgres-request-media"
    })
    .to_string();
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT pg_notify('scope_repo_changes', $1)",
        [payload.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}
