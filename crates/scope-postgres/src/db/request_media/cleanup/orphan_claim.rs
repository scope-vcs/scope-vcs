use super::support::orphan_object_keys;
use crate::{
    db::request_media::persistence::{as_i32, as_i64},
    error::PostgresError,
};
use scope_domain::requests::attachments::RequestAttachmentCleanupLease;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

pub(super) async fn claim_orphan_objects(
    db: &sea_orm::DatabaseConnection,
    lease_token: &str,
    now_unix: u64,
    lease_expires_at_unix: u64,
) -> Result<Option<RequestAttachmentCleanupLease>, PostgresError> {
    loop {
        match try_claim_orphan_objects(db, lease_token, now_unix, lease_expires_at_unix).await? {
            OrphanClaim::Claimed(lease) => return Ok(Some(lease)),
            OrphanClaim::Empty => return Ok(None),
            OrphanClaim::Contended => {}
        }
    }
}

enum OrphanClaim {
    Claimed(RequestAttachmentCleanupLease),
    Contended,
    Empty,
}

async fn try_claim_orphan_objects(
    db: &sea_orm::DatabaseConnection,
    lease_token: &str,
    now_unix: u64,
    lease_expires_at_unix: u64,
) -> Result<OrphanClaim, PostgresError> {
    let tx = db.begin().await?;
    let candidate = tx
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT attachment.id
             FROM scope_request_media_attachments attachment
             WHERE NOT EXISTS (
                    SELECT 1 FROM scope_request_media_cleanup_jobs cleanup
                    WHERE cleanup.attachment_id = attachment.id
               ) AND NOT EXISTS (
                    SELECT 1 FROM scope_request_media_orphan_cleanup_leases lease
                    WHERE lease.attachment_id = attachment.id
                      AND lease.lease_expires_at_unix > $1
               ) AND (
                    EXISTS (SELECT 1 FROM scope_request_media_processing_objects object
                            WHERE object.attachment_id = attachment.id AND object.state = 'Orphaned')
                    OR EXISTS (SELECT 1 FROM scope_request_media_abandoned_objects object
                               WHERE object.attachment_id = attachment.id AND object.deleted_at_unix IS NULL)
               )
             ORDER BY attachment.id FOR UPDATE OF attachment SKIP LOCKED LIMIT 1",
            [as_i64(now_unix, "orphan cleanup claim time")?.into()],
        ))
        .await?;
    let Some(candidate) = candidate else {
        tx.commit().await?;
        return Ok(OrphanClaim::Empty);
    };
    let attachment_id = candidate.try_get::<String>("", "id")?;
    let existing = tx
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT * FROM scope_request_media_orphan_cleanup_leases
             WHERE attachment_id = $1 FOR UPDATE",
            [attachment_id.clone().into()],
        ))
        .await?;
    let (generation, attempt) = if let Some(existing) = existing {
        let expires = u64::try_from(existing.try_get::<i64>("", "lease_expires_at_unix")?)
            .map_err(PostgresError::internal)?;
        if expires > now_unix {
            tx.commit().await?;
            return Ok(OrphanClaim::Contended);
        }
        (
            u64::try_from(existing.try_get::<i64>("", "lease_generation")?)
                .map_err(PostgresError::internal)?
                .checked_add(1)
                .ok_or_else(|| PostgresError::internal_message("cleanup generation overflow"))?,
            u32::try_from(existing.try_get::<i32>("", "attempt")?)
                .map_err(PostgresError::internal)?
                .checked_add(1)
                .ok_or_else(|| PostgresError::internal_message("cleanup attempt overflow"))?,
        )
    } else {
        (1, 1)
    };
    tx.execute_raw(Statement::from_sql_and_values(
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
    .await?;
    let object_keys = orphan_object_keys(&tx, &attachment_id).await?;
    tx.commit().await?;
    Ok(OrphanClaim::Claimed(RequestAttachmentCleanupLease {
        attachment_id,
        object_keys,
        lease_token: lease_token.to_string(),
        lease_generation: generation,
        attempt,
    }))
}
