use crate::lease::{HeartbeatError, supervise_lease};
use crate::{
    config::WorkerSettings,
    health::WorkerHealth,
    runtime::{
        db_error, dependencies_ready, lease_expiry, random_id, retry_delay, wait_or_shutdown,
    },
};
use scope_media_storage::MediaStorage;
use scope_postgres::db::{MediaLeaseMutation, MetadataStore};

pub async fn run(
    metadata: MetadataStore,
    storage: MediaStorage,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    loop {
        if !dependencies_ready(&metadata, &storage, &health).await {
            if wait_or_shutdown(settings.poll_interval).await {
                return Ok(());
            }
            continue;
        }
        let result = cleanup_next_job(&metadata, &storage, &settings, &health).await;
        let should_wait = should_wait_after_poll(&result);
        match &result {
            Ok(CleanupOutcome::NoJob) => {}
            Ok(outcome) => tracing::info!(?outcome, "media cleanup job finished"),
            Err(error) => tracing::error!(%error, "media cleanup poll failed"),
        }
        health.mark_cleanup_poll(crate::unix_now()?);
        if !should_wait {
            continue;
        }
        if wait_or_shutdown(settings.poll_interval).await {
            return Ok(());
        }
    }
}

fn should_wait_after_poll(result: &anyhow::Result<CleanupOutcome>) -> bool {
    matches!(result, Ok(CleanupOutcome::NoJob) | Err(_))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CleanupOutcome {
    NoJob,
    Completed,
    Failed,
    LeaseLost,
}

async fn cleanup_next_job(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    settings: &WorkerSettings,
    health: &WorkerHealth,
) -> anyhow::Result<CleanupOutcome> {
    let now = crate::unix_now()?;
    metadata
        .media()
        .enqueue_expired_attachment_cleanup(now)
        .await
        .map_err(db_error)?;
    let token = random_id("cleanup")?;
    let Some(lease) = metadata
        .media()
        .claim_cleanup_job(&token, now, lease_expiry(now, settings.lease_duration)?)
        .await
        .map_err(db_error)?
    else {
        return Ok(CleanupOutcome::NoJob);
    };
    let _activity = health.cleanup_activity();
    for key in &lease.object_keys {
        let deletion = storage.delete_object_key(key);
        tokio::pin!(deletion);
        match supervise_lease(&mut deletion, settings.lease_duration, || async {
            let now = crate::unix_now()?;
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
                .map_err(db_error)
        })
        .await
        {
            Ok(Ok(())) => {}
            Err(HeartbeatError::LeaseLost) => {
                // Finish the in-flight delete before abandoning this lease.
                let _ = deletion.await;
                return Ok(CleanupOutcome::LeaseLost);
            }
            Err(HeartbeatError::Database(error)) => return Err(error),
            Ok(Err(error)) => {
                tracing::warn!(
                    attachment_id = %lease.attachment_id,
                    object_key = %key,
                    %error,
                    "media object cleanup failed"
                );
                let now = crate::unix_now()?;
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
                    .await
                    .map_err(db_error)?;
                return Ok(match result {
                    MediaLeaseMutation::Applied(()) => CleanupOutcome::Failed,
                    MediaLeaseMutation::LeaseLost => CleanupOutcome::LeaseLost,
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
            crate::unix_now()?,
        )
        .await
        .map_err(db_error)?;
    Ok(match result {
        MediaLeaseMutation::Applied(()) => CleanupOutcome::Completed,
        MediaLeaseMutation::LeaseLost => CleanupOutcome::LeaseLost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_waits_when_idle_or_after_an_error() {
        assert!(should_wait_after_poll(&Ok(CleanupOutcome::NoJob)));
        assert!(should_wait_after_poll(&Err(anyhow::anyhow!(
            "database unavailable"
        ))));
        assert!(!should_wait_after_poll(&Ok(CleanupOutcome::Completed)));
        assert!(!should_wait_after_poll(&Ok(CleanupOutcome::Failed)));
        assert!(!should_wait_after_poll(&Ok(CleanupOutcome::LeaseLost)));
    }
}
