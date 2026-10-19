use crate::lease::{HeartbeatError, supervise_lease};
use crate::{
    config::WorkerSettings,
    health::{MediaWorkerLoop, WorkerHealth},
    runtime::{Cadence, JobLoop, JobOutcome, lease_expiry, retry_delay},
};
use scope_domain::requests::attachments::{
    REQUEST_MEDIA_LATE_WRITE_GRACE_SECONDS, RequestAttachmentCleanupLease,
};
use scope_media_storage::{MEDIA_PUT_TIMEOUT, MediaStorage};
use scope_postgres::db::{MediaLeaseMutation, MetadataStore};
use scope_service_runtime::{random_hex, unix_now};
use std::time::Duration;
use tracing::Instrument as _;

const EXPIRY_DISCOVERY_INTERVAL: Duration = Duration::from_secs(60);
const _: () = assert!(MEDIA_PUT_TIMEOUT.as_secs() <= REQUEST_MEDIA_LATE_WRITE_GRACE_SECONDS);

pub async fn run(
    metadata: MetadataStore,
    storage: MediaStorage,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    let expiry_discovery = Cadence::new(EXPIRY_DISCOVERY_INTERVAL);
    JobLoop {
        worker_loop: MediaWorkerLoop::Cleanup,
        metadata: &metadata,
        storage: &storage,
        health: &health,
        poll_interval: settings.poll_interval,
    }
    .run(|| cleanup_next_job(&metadata, &storage, &settings, &health, &expiry_discovery))
    .await
}

async fn cleanup_next_job(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    settings: &WorkerSettings,
    health: &WorkerHealth,
    expiry_discovery: &Cadence,
) -> anyhow::Result<JobOutcome> {
    let now = unix_now()?;
    if expiry_discovery.due(now) {
        metadata
            .media()
            .enqueue_expired_attachment_cleanup(now)
            .await?;
        expiry_discovery.completed(now);
    }
    let token = random_hex("cleanup-", 16)?;
    let Some((lease, queued_trace)) = metadata
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
    queued_trace.link(&span);
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
    let _activity = health.activity(MediaWorkerLoop::Cleanup);
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
        .complete_cleanup_job(lease, unix_now()?)
        .await?;
    Ok(match result {
        MediaLeaseMutation::Applied(()) => JobOutcome::Completed,
        MediaLeaseMutation::LeaseLost => JobOutcome::LeaseLost,
    })
}
