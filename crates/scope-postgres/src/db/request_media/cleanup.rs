mod support;

use super::{
    MediaLeaseMutation, MediaStore, RequestAttachmentCleanupReason,
    persistence::{as_i32, as_i64},
};
use crate::{db::locks::acquire_shared_repository_lock, error::PostgresError};
use scope_domain::requests::attachments::{RequestAttachmentCleanupLease, validate_cleanup_lease};
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, TransactionTrait};

use support::{
    all_attachment_object_keys, attachment_cleanup_is_due, cancel_processing_and_orphan_outputs,
    cleanup_available_at, lock_attachment_row, lock_processing_job_if_present,
    mark_all_inventory_deleted, mark_orphan_inventory_deleted, orphan_object_keys,
    reconcile_completed_inventories,
};

const LATE_WRITE_GRACE_SECONDS: u64 = 60;
const TOMBSTONE_RECONCILIATION_SECONDS: u64 = 24 * 60 * 60;

impl MediaStore {
    /// Discovers expired upload/draft media and periodically reopens completed
    /// tombstones so a storage write that finished after its lease can never
    /// resurrect bytes permanently.
    pub async fn enqueue_expired_attachment_cleanup(
        &self,
        now_unix: u64,
    ) -> Result<u64, PostgresError> {
        reconcile_completed_inventories(self.db.as_ref(), now_unix).await?;
        let candidates = self
            .db
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT attachment.id, attachment.repository_id,
                        CASE WHEN attachment.state = 'Prepared'
                             THEN 'IncompleteUploadExpired'
                             ELSE 'UnboundDraftExpired' END AS reason
                 FROM scope_request_media_attachments attachment
                 WHERE (
                    (attachment.state = 'Prepared' AND attachment.upload_expires_at_unix <= $1)
                    OR (attachment.state <> 'Prepared'
                        AND attachment.unbound_expires_at_unix IS NOT NULL
                        AND attachment.unbound_expires_at_unix <= $1
                        AND NOT EXISTS (
                            SELECT 1 FROM scope_request_media_bindings binding
                            WHERE binding.attachment_id = attachment.id
                        ))
                 )
                   AND NOT EXISTS (
                        SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                        WHERE cleanup.attachment_id = attachment.id
                   )
                 ORDER BY attachment.repository_id, attachment.id",
                [as_i64(now_unix, "cleanup discovery time")?.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        let mut inserted = 0_u64;
        for candidate in candidates {
            let attachment_id = candidate
                .try_get::<String>("", "id")
                .map_err(PostgresError::internal)?;
            let repository_id = candidate
                .try_get::<String>("", "repository_id")
                .map_err(PostgresError::internal)?;
            let reason = candidate
                .try_get::<String>("", "reason")
                .map_err(PostgresError::internal)?;
            let tx = self.db.begin().await.map_err(PostgresError::internal)?;
            acquire_shared_repository_lock(&tx, &repository_id).await?;
            lock_processing_job_if_present(&tx, &attachment_id).await?;
            let Some(row) = lock_attachment_row(&tx, &attachment_id).await? else {
                tx.commit().await.map_err(PostgresError::internal)?;
                continue;
            };
            if !attachment_cleanup_is_due(&tx, &row, now_unix).await? {
                tx.commit().await.map_err(PostgresError::internal)?;
                continue;
            }
            let available_at = cleanup_available_at(&tx, &attachment_id, now_unix).await?;
            let result = tx
                .execute(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "INSERT INTO scope_request_media_cleanup_jobs (
                        attachment_id, repository_id, reason, state, available_at_unix,
                        created_at_unix, updated_at_unix
                     ) VALUES ($1, $2, $3, 'Queued', $4, $5, $5)
                     ON CONFLICT (attachment_id) DO NOTHING",
                    [
                        attachment_id.clone().into(),
                        repository_id.into(),
                        reason.into(),
                        as_i64(available_at, "cleanup available time")?.into(),
                        as_i64(now_unix, "cleanup creation time")?.into(),
                    ],
                ))
                .await
                .map_err(PostgresError::internal)?;
            if result.rows_affected() == 1 {
                cancel_processing_and_orphan_outputs(&tx, &attachment_id, now_unix).await?;
                inserted = inserted.saturating_add(1);
            }
            tx.commit().await.map_err(PostgresError::internal)?;
        }
        Ok(inserted)
    }

    pub async fn claim_cleanup_job(
        &self,
        lease_token: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<Option<RequestAttachmentCleanupLease>, PostgresError> {
        validate_new_cleanup_lease(lease_token, now_unix, lease_expires_at_unix)?;
        if let Some(lease) = claim_attachment_tombstone(
            self.db.as_ref(),
            lease_token,
            now_unix,
            lease_expires_at_unix,
        )
        .await?
        {
            return Ok(Some(lease));
        }
        claim_orphan_objects(
            self.db.as_ref(),
            lease_token,
            now_unix,
            lease_expires_at_unix,
        )
        .await
    }

    pub async fn renew_cleanup_lease(
        &self,
        attachment_id: &str,
        lease_token: &str,
        lease_generation: u64,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<bool, PostgresError> {
        validate_new_cleanup_lease(lease_token, now_unix, lease_expires_at_unix)?;
        let updated = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_cleanup_jobs
                 SET lease_expires_at_unix = $5, updated_at_unix = $4
                 WHERE attachment_id = $1 AND state = 'Leased' AND lease_token = $2
                   AND lease_generation = $3 AND lease_expires_at_unix > $4",
                [
                    attachment_id.into(),
                    lease_token.into(),
                    as_i64(lease_generation, "cleanup lease generation")?.into(),
                    as_i64(now_unix, "cleanup heartbeat time")?.into(),
                    as_i64(lease_expires_at_unix, "cleanup lease expiry")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        if updated.rows_affected() == 1 {
            return Ok(true);
        }
        let orphan_updated = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_orphan_cleanup_leases
                 SET lease_expires_at_unix = $5, updated_at_unix = $4
                 WHERE attachment_id = $1 AND lease_token = $2 AND lease_generation = $3
                   AND lease_expires_at_unix > $4",
                [
                    attachment_id.into(),
                    lease_token.into(),
                    as_i64(lease_generation, "cleanup lease generation")?.into(),
                    as_i64(now_unix, "cleanup heartbeat time")?.into(),
                    as_i64(lease_expires_at_unix, "cleanup lease expiry")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(orphan_updated.rows_affected() == 1)
    }

    pub async fn complete_cleanup_job(
        &self,
        attachment_id: &str,
        lease_token: &str,
        lease_generation: u64,
        now_unix: u64,
    ) -> Result<MediaLeaseMutation<()>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        if let Some(lease) =
            locked_attachment_cleanup_lease(&tx, attachment_id, lease_token, lease_generation)
                .await?
        {
            if validate_cleanup_lease(
                attachment_id,
                &lease,
                lease_token,
                lease_generation,
                now_unix,
            )
            .is_err()
            {
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(MediaLeaseMutation::LeaseLost);
            }
            // The write grace is checked again while holding the tombstone row.
            if cleanup_available_at(&tx, attachment_id, now_unix).await? > now_unix {
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(MediaLeaseMutation::LeaseLost);
            }
            tx.execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_request_media_cleanup_jobs
                 SET state = 'Completed', lease_token = NULL, lease_expires_at_unix = NULL,
                     completed_at_unix = $2, updated_at_unix = $2
                 WHERE attachment_id = $1",
                [
                    attachment_id.into(),
                    as_i64(now_unix, "cleanup completion time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
            mark_all_inventory_deleted(&tx, attachment_id, now_unix).await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(MediaLeaseMutation::Applied(()));
        }
        if let Some(lease) =
            locked_orphan_cleanup_lease(&tx, attachment_id, lease_token, lease_generation).await?
        {
            if validate_cleanup_lease(
                attachment_id,
                &lease,
                lease_token,
                lease_generation,
                now_unix,
            )
            .is_err()
            {
                tx.commit().await.map_err(PostgresError::internal)?;
                return Ok(MediaLeaseMutation::LeaseLost);
            }
            mark_orphan_inventory_deleted(&tx, attachment_id, now_unix).await?;
            tx.execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "DELETE FROM scope_request_media_orphan_cleanup_leases WHERE attachment_id = $1",
                [attachment_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(MediaLeaseMutation::Applied(()));
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(MediaLeaseMutation::LeaseLost)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn fail_cleanup_job(
        &self,
        attachment_id: &str,
        lease_token: &str,
        lease_generation: u64,
        now_unix: u64,
        retry_at_unix: u64,
        error: &str,
    ) -> Result<MediaLeaseMutation<()>, PostgresError> {
        if retry_at_unix < now_unix {
            return Err(PostgresError::invalid_input(
                "cleanup retry time cannot be in the past",
            ));
        }
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        if locked_attachment_cleanup_lease(&tx, attachment_id, lease_token, lease_generation)
            .await?
            .is_some()
        {
            let result = tx
                .execute(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "UPDATE scope_request_media_cleanup_jobs
                     SET state = 'Queued', available_at_unix = $5, lease_token = NULL,
                         lease_expires_at_unix = NULL, last_error = $6, updated_at_unix = $4
                     WHERE attachment_id = $1 AND state = 'Leased' AND lease_token = $2
                       AND lease_generation = $3 AND lease_expires_at_unix > $4",
                    [
                        attachment_id.into(),
                        lease_token.into(),
                        as_i64(lease_generation, "cleanup lease generation")?.into(),
                        as_i64(now_unix, "cleanup failure time")?.into(),
                        as_i64(retry_at_unix, "cleanup retry time")?.into(),
                        error.into(),
                    ],
                ))
                .await
                .map_err(PostgresError::internal)?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(if result.rows_affected() == 1 {
                MediaLeaseMutation::Applied(())
            } else {
                MediaLeaseMutation::LeaseLost
            });
        }
        if locked_orphan_cleanup_lease(&tx, attachment_id, lease_token, lease_generation)
            .await?
            .is_some()
        {
            let result = tx
                .execute(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "UPDATE scope_request_media_orphan_cleanup_leases
                 SET lease_token = '', lease_expires_at_unix = $5, updated_at_unix = $4
                 WHERE attachment_id = $1 AND lease_token = $2 AND lease_generation = $3
                   AND lease_expires_at_unix > $4",
                    [
                        attachment_id.into(),
                        lease_token.into(),
                        as_i64(lease_generation, "cleanup lease generation")?.into(),
                        as_i64(now_unix, "cleanup failure time")?.into(),
                        as_i64(retry_at_unix, "cleanup retry time")?.into(),
                    ],
                ))
                .await
                .map_err(PostgresError::internal)?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(if result.rows_affected() == 1 {
                MediaLeaseMutation::Applied(())
            } else {
                MediaLeaseMutation::LeaseLost
            });
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(MediaLeaseMutation::LeaseLost)
    }
}

pub(crate) async fn tombstone_repository_attachments<C>(
    conn: &C,
    repository_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    tombstone_matching_attachments(
        conn,
        "repository_id",
        repository_id,
        RequestAttachmentCleanupReason::RepositoryDeleted,
        now_unix,
    )
    .await
}

pub(crate) async fn tombstone_request_attachments<C>(
    conn: &C,
    request_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    tombstone_matching_attachments(
        conn,
        "request_id",
        request_id,
        RequestAttachmentCleanupReason::RequestDeleted,
        now_unix,
    )
    .await
}

async fn tombstone_matching_attachments<C>(
    conn: &C,
    column: &str,
    id: &str,
    reason: RequestAttachmentCleanupReason,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let observed = conn
        .query_all(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT id, repository_id FROM scope_request_media_attachments
                 WHERE {column} = $1 ORDER BY id"
            ),
            [id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    let attachment_ids = observed
        .into_iter()
        .map(|row| {
            row.try_get::<String>("", "id")
                .map_err(PostgresError::internal)
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Processing jobs and attachments use one global order everywhere media
    // lifecycle work overlaps: every job first, then every attachment, with
    // identifiers sorted inside each class.
    for attachment_id in &attachment_ids {
        lock_processing_job_if_present(conn, attachment_id).await?;
    }
    let mut locked_attachments = Vec::with_capacity(attachment_ids.len());
    for attachment_id in &attachment_ids {
        let Some(row) = lock_attachment_row(conn, attachment_id).await? else {
            continue;
        };
        if row
            .try_get::<String>("", column)
            .map_err(PostgresError::internal)?
            != id
        {
            continue;
        }
        locked_attachments.push((
            attachment_id.clone(),
            row.try_get::<String>("", "repository_id")
                .map_err(PostgresError::internal)?,
        ));
    }

    for (attachment_id, repository_id) in locked_attachments {
        let available_at = cleanup_available_at(conn, &attachment_id, now_unix).await?;
        conn.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_request_media_cleanup_jobs (
                attachment_id, repository_id, reason, state, available_at_unix,
                created_at_unix, updated_at_unix
             ) VALUES ($1, $2, $3, 'Queued', $4, $5, $5)
             ON CONFLICT (attachment_id) DO UPDATE
             SET reason = CASE
                    WHEN scope_request_media_cleanup_jobs.reason = 'RepositoryDeleted'
                        THEN scope_request_media_cleanup_jobs.reason
                    WHEN EXCLUDED.reason = 'RepositoryDeleted' THEN EXCLUDED.reason
                    WHEN scope_request_media_cleanup_jobs.reason = 'RequestDeleted'
                        THEN scope_request_media_cleanup_jobs.reason
                    WHEN EXCLUDED.reason = 'RequestDeleted' THEN EXCLUDED.reason
                    ELSE scope_request_media_cleanup_jobs.reason END,
                 state = CASE
                    WHEN scope_request_media_cleanup_jobs.state = 'Completed' THEN 'Queued'
                    ELSE scope_request_media_cleanup_jobs.state END,
                 available_at_unix = GREATEST(
                    scope_request_media_cleanup_jobs.available_at_unix,
                    EXCLUDED.available_at_unix
                 ),
                 completed_at_unix = NULL,
                 updated_at_unix = EXCLUDED.updated_at_unix",
            [
                attachment_id.clone().into(),
                repository_id.into(),
                reason.as_str().into(),
                as_i64(available_at, "cleanup available time")?.into(),
                as_i64(now_unix, "cleanup creation time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        cancel_processing_and_orphan_outputs(conn, &attachment_id, now_unix).await?;
    }
    Ok(())
}

async fn claim_attachment_tombstone(
    db: &sea_orm::DatabaseConnection,
    lease_token: &str,
    now_unix: u64,
    lease_expires_at_unix: u64,
) -> Result<Option<RequestAttachmentCleanupLease>, PostgresError> {
    let tx = db.begin().await.map_err(PostgresError::internal)?;
    let candidate = tx
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT cleanup.attachment_id, cleanup.repository_id
             FROM scope_request_media_cleanup_jobs cleanup
             WHERE ((cleanup.state = 'Queued' AND cleanup.available_at_unix <= $1)
                    OR (cleanup.state = 'Leased' AND cleanup.lease_expires_at_unix <= $1))
             ORDER BY cleanup.available_at_unix, cleanup.created_at_unix, cleanup.attachment_id
             LIMIT 1",
            [as_i64(now_unix, "cleanup claim time")?.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    let Some(candidate) = candidate else {
        tx.commit().await.map_err(PostgresError::internal)?;
        return Ok(None);
    };
    let attachment_id = candidate
        .try_get::<String>("", "attachment_id")
        .map_err(PostgresError::internal)?;
    let observed_repository_id = candidate
        .try_get::<String>("", "repository_id")
        .map_err(PostgresError::internal)?;
    acquire_shared_repository_lock(&tx, &observed_repository_id).await?;
    lock_processing_job_if_present(&tx, &attachment_id).await?;
    let Some(attachment) = lock_attachment_row(&tx, &attachment_id).await? else {
        tx.commit().await.map_err(PostgresError::internal)?;
        return Ok(None);
    };
    let repository_id = attachment
        .try_get::<String>("", "repository_id")
        .map_err(PostgresError::internal)?;
    if repository_id != observed_repository_id {
        tx.commit().await.map_err(PostgresError::internal)?;
        return Ok(None);
    }
    let row = tx
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM scope_request_media_cleanup_jobs
             WHERE attachment_id = $1
               AND ((state = 'Queued' AND available_at_unix <= $2)
                    OR (state = 'Leased' AND lease_expires_at_unix <= $2))
             FOR UPDATE SKIP LOCKED",
            [
                attachment_id.clone().into(),
                as_i64(now_unix, "cleanup claim time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    let Some(row) = row else {
        tx.commit().await.map_err(PostgresError::internal)?;
        return Ok(None);
    };
    let available_at = cleanup_available_at(&tx, &attachment_id, now_unix).await?;
    if available_at > now_unix {
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_request_media_cleanup_jobs
             SET state = 'Queued', available_at_unix = $2, lease_token = NULL,
                 lease_expires_at_unix = NULL, updated_at_unix = $3
             WHERE attachment_id = $1",
            [
                attachment_id.clone().into(),
                as_i64(available_at, "cleanup available time")?.into(),
                as_i64(now_unix, "cleanup deferral time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        return Ok(None);
    }
    let generation = u64::try_from(
        row.try_get::<i64>("", "lease_generation")
            .map_err(PostgresError::internal)?,
    )
    .map_err(PostgresError::internal)?
    .checked_add(1)
    .ok_or_else(|| PostgresError::internal_message("cleanup generation overflow"))?;
    let attempt = u32::try_from(
        row.try_get::<i32>("", "attempt")
            .map_err(PostgresError::internal)?,
    )
    .map_err(PostgresError::internal)?
    .checked_add(1)
    .ok_or_else(|| PostgresError::internal_message("cleanup attempt overflow"))?;
    tx.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_media_cleanup_jobs
         SET state = 'Leased', lease_token = $2, lease_generation = $3,
             lease_expires_at_unix = $4, attempt = $5, updated_at_unix = $6
         WHERE attachment_id = $1",
        [
            attachment_id.clone().into(),
            lease_token.into(),
            as_i64(generation, "cleanup generation")?.into(),
            as_i64(lease_expires_at_unix, "cleanup expiry")?.into(),
            as_i32(attempt, "cleanup attempt")?.into(),
            as_i64(now_unix, "cleanup claim time")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    let object_keys = all_attachment_object_keys(&tx, &attachment_id).await?;
    tx.commit().await.map_err(PostgresError::internal)?;
    Ok(Some(RequestAttachmentCleanupLease {
        attachment_id,
        repository_id,
        object_keys,
        lease_token: lease_token.to_string(),
        lease_generation: generation,
        attempt,
        lease_expires_at_unix,
    }))
}

async fn claim_orphan_objects(
    db: &sea_orm::DatabaseConnection,
    lease_token: &str,
    now_unix: u64,
    lease_expires_at_unix: u64,
) -> Result<Option<RequestAttachmentCleanupLease>, PostgresError> {
    let tx = db.begin().await.map_err(PostgresError::internal)?;
    let candidate = tx
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT attachment.id, attachment.repository_id
             FROM scope_request_media_attachments attachment
             WHERE NOT EXISTS (
                    SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                    WHERE cleanup.attachment_id = attachment.id
               ) AND (
                    EXISTS (SELECT 1 FROM scope_request_media_processing_objects object
                            WHERE object.attachment_id = attachment.id AND object.state = 'Orphaned')
                    OR EXISTS (SELECT 1 FROM scope_request_media_abandoned_objects object
                               WHERE object.attachment_id = attachment.id AND object.deleted_at_unix IS NULL)
               )
             ORDER BY attachment.id LIMIT 1"
                .to_string(),
        ))
        .await
        .map_err(PostgresError::internal)?;
    let Some(candidate) = candidate else {
        tx.commit().await.map_err(PostgresError::internal)?;
        return Ok(None);
    };
    let attachment_id = candidate
        .try_get::<String>("", "id")
        .map_err(PostgresError::internal)?;
    tx.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT pg_advisory_xact_lock(hashtextextended('scope:request-media-orphan:' || $1, 0))",
        [attachment_id.clone().into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    let existing = tx
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM scope_request_media_orphan_cleanup_leases
             WHERE attachment_id = $1 FOR UPDATE",
            [attachment_id.clone().into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    let (generation, attempt) = if let Some(existing) = existing {
        let expires = u64::try_from(
            existing
                .try_get::<i64>("", "lease_expires_at_unix")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?;
        if expires > now_unix {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        }
        (
            u64::try_from(
                existing
                    .try_get::<i64>("", "lease_generation")
                    .map_err(PostgresError::internal)?,
            )
            .map_err(PostgresError::internal)?
            .checked_add(1)
            .ok_or_else(|| PostgresError::internal_message("cleanup generation overflow"))?,
            u32::try_from(
                existing
                    .try_get::<i32>("", "attempt")
                    .map_err(PostgresError::internal)?,
            )
            .map_err(PostgresError::internal)?
            .checked_add(1)
            .ok_or_else(|| PostgresError::internal_message("cleanup attempt overflow"))?,
        )
    } else {
        (1, 1)
    };
    tx.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_request_media_orphan_cleanup_leases (
            attachment_id, lease_token, lease_generation, lease_expires_at_unix,
            attempt, updated_at_unix
         ) VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (attachment_id) DO UPDATE SET
            lease_token = EXCLUDED.lease_token,
            lease_generation = EXCLUDED.lease_generation,
            lease_expires_at_unix = EXCLUDED.lease_expires_at_unix,
            attempt = EXCLUDED.attempt,
            updated_at_unix = EXCLUDED.updated_at_unix",
        [
            attachment_id.clone().into(),
            lease_token.into(),
            as_i64(generation, "cleanup generation")?.into(),
            as_i64(lease_expires_at_unix, "cleanup expiry")?.into(),
            as_i32(attempt, "cleanup attempt")?.into(),
            as_i64(now_unix, "cleanup claim time")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    let repository_id = candidate
        .try_get::<String>("", "repository_id")
        .map_err(PostgresError::internal)?;
    let object_keys = orphan_object_keys(&tx, &attachment_id).await?;
    tx.commit().await.map_err(PostgresError::internal)?;
    Ok(Some(RequestAttachmentCleanupLease {
        attachment_id,
        repository_id,
        object_keys,
        lease_token: lease_token.to_string(),
        lease_generation: generation,
        attempt,
        lease_expires_at_unix,
    }))
}

async fn locked_attachment_cleanup_lease<C>(
    conn: &C,
    attachment_id: &str,
    lease_token: &str,
    lease_generation: u64,
) -> Result<Option<RequestAttachmentCleanupLease>, PostgresError>
where
    C: ConnectionTrait,
{
    let row = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM scope_request_media_cleanup_jobs
             WHERE attachment_id = $1 AND state = 'Leased' AND lease_token = $2
               AND lease_generation = $3 FOR UPDATE",
            [
                attachment_id.into(),
                lease_token.into(),
                as_i64(lease_generation, "cleanup generation")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    match row {
        Some(row) => cleanup_lease_from_row(conn, row, true).await.map(Some),
        None => Ok(None),
    }
}

async fn locked_orphan_cleanup_lease<C>(
    conn: &C,
    attachment_id: &str,
    lease_token: &str,
    lease_generation: u64,
) -> Result<Option<RequestAttachmentCleanupLease>, PostgresError>
where
    C: ConnectionTrait,
{
    let row = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT lease.*, attachment.repository_id
             FROM scope_request_media_orphan_cleanup_leases lease
             JOIN scope_request_media_attachments attachment ON attachment.id = lease.attachment_id
             WHERE lease.attachment_id = $1 AND lease.lease_token = $2
               AND lease.lease_generation = $3 FOR UPDATE OF lease",
            [
                attachment_id.into(),
                lease_token.into(),
                as_i64(lease_generation, "cleanup generation")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    match row {
        Some(row) => cleanup_lease_from_row(conn, row, false).await.map(Some),
        None => Ok(None),
    }
}

async fn cleanup_lease_from_row<C>(
    conn: &C,
    row: QueryResult,
    tombstone: bool,
) -> Result<RequestAttachmentCleanupLease, PostgresError>
where
    C: ConnectionTrait,
{
    let attachment_id = row
        .try_get::<String>("", "attachment_id")
        .map_err(PostgresError::internal)?;
    let object_keys = if tombstone {
        all_attachment_object_keys(conn, &attachment_id).await?
    } else {
        orphan_object_keys(conn, &attachment_id).await?
    };
    Ok(RequestAttachmentCleanupLease {
        attachment_id,
        repository_id: row
            .try_get("", "repository_id")
            .map_err(PostgresError::internal)?,
        object_keys,
        lease_token: row
            .try_get("", "lease_token")
            .map_err(PostgresError::internal)?,
        lease_generation: u64::try_from(
            row.try_get::<i64>("", "lease_generation")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?,
        attempt: u32::try_from(
            row.try_get::<i32>("", "attempt")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?,
        lease_expires_at_unix: u64::try_from(
            row.try_get::<i64>("", "lease_expires_at_unix")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?,
    })
}

fn validate_new_cleanup_lease(
    lease_token: &str,
    now_unix: u64,
    lease_expires_at_unix: u64,
) -> Result<(), PostgresError> {
    if lease_token.trim().is_empty() || lease_expires_at_unix <= now_unix {
        return Err(PostgresError::invalid_input(
            "cleanup lease must have a token and future expiry",
        ));
    }
    Ok(())
}
