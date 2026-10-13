use crate::lease::{HeartbeatError, supervise_lease};
use crate::{
    config::WorkerSettings,
    health::WorkerHealth,
    runtime::{JobLoop, JobOutcome, lease_expiry, retry_delay},
};
use scope_domain::requests::attachments::RequestAttachmentCleanupLease;
use scope_media_storage::MediaStorage;
use scope_postgres::db::{MediaLeaseMutation, MetadataStore};
use scope_service_runtime::{random_hex, unix_now};
use tracing::Instrument as _;

pub async fn run(
    metadata: MetadataStore,
    storage: MediaStorage,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    JobLoop {
        kind: "cleanup",
        metadata: &metadata,
        storage: &storage,
        health: &health,
        poll_interval: settings.poll_interval,
        mark_poll: WorkerHealth::mark_cleanup_poll,
    }
    .run(|| cleanup_next_job(&metadata, &storage, &settings, &health))
    .await
}

async fn cleanup_next_job(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    settings: &WorkerSettings,
    health: &WorkerHealth,
) -> anyhow::Result<JobOutcome> {
    let now = unix_now()?;
    metadata
        .media()
        .enqueue_expired_attachment_cleanup(now)
        .await?;
    let token = random_hex("cleanup-", 16)?;
    let Some(lease) = metadata
        .media()
        .claim_cleanup_job(&token, now, lease_expiry(now, settings.lease_duration)?)
        .await?
    else {
        return Ok(JobOutcome::NoJob);
    };
    let span = tracing::info_span!(
        parent: None,
        "job.media_cleanup",
        otel.kind = "consumer",
        scope.job.kind = "media_cleanup",
        scope.job.id = %lease.attachment_id,
        scope.job.attempt = lease.attempt,
    );
    clean_up_lease(metadata, storage, settings, health, &lease)
        .instrument(span)
        .await
}

async fn clean_up_lease(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    settings: &WorkerSettings,
    health: &WorkerHealth,
    lease: &RequestAttachmentCleanupLease,
) -> anyhow::Result<JobOutcome> {
    let _activity = health.cleanup_activity();
    for key in &lease.object_keys {
        let deletion = storage.delete_object_key(key);
        tokio::pin!(deletion);
        match supervise_lease(&mut deletion, settings.lease_duration, || async {
            let now = unix_now()?;
            metadata
                .media()
                .renew_cleanup_lease(
                    &lease.attachment_id,
                    &lease.lease_token,
                    lease.lease_generation,
                    now,
                    lease_expiry(now, settings.lease_duration)?,
                )
                .await
                .map_err(anyhow::Error::from)
        })
        .await
        {
            Ok(Ok(())) => {}
            Err(HeartbeatError::LeaseLost) => {
                let _ = deletion.await;
                return Ok(JobOutcome::LeaseLost);
            }
            Err(HeartbeatError::Database(error)) => return Err(error),
            Ok(Err(error)) => {
                tracing::warn!(
                    attachment_id = %lease.attachment_id,
                    object_key = %key,
                    %error,
                    "media object cleanup failed"
                );
                let now = unix_now()?;
                let result = metadata
                    .media()
                    .fail_cleanup_job(
                        &lease.attachment_id,
                        &lease.lease_token,
                        lease.lease_generation,
                        now,
                        now.saturating_add(retry_delay(lease.attempt).as_secs()),
                        "Media object deletion failed; retry scheduled.",
                    )
                    .await?;
                return Ok(match result {
                    MediaLeaseMutation::Applied(()) => JobOutcome::Failed,
                    MediaLeaseMutation::LeaseLost => JobOutcome::LeaseLost,
                });
            }
        }
    }
    let result = metadata
        .media()
        .complete_cleanup_job(
            &lease.attachment_id,
            &lease.lease_token,
            lease.lease_generation,
            unix_now()?,
        )
        .await?;
    Ok(match result {
        MediaLeaseMutation::Applied(()) => JobOutcome::Completed,
        MediaLeaseMutation::LeaseLost => JobOutcome::LeaseLost,
    })
}
