use crate::{config::WorkerSettings, health::WorkerHealth};
use scope_domain::requests::attachments::RequestAttachmentCleanupLease;
use scope_media_storage::MediaStorage;
use scope_postgres::db::{MediaLeaseMutation, MetadataStore};
use std::{future::Future, time::Duration};

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
        match with_heartbeat(
            metadata,
            &lease,
            settings.lease_duration,
            storage.delete_object_key(key),
        )
        .await
        {
            Ok(()) => {}
            Err(StepError::LeaseLost) => return Ok(CleanupOutcome::LeaseLost),
            Err(StepError::Database(error)) => return Err(error),
            Err(StepError::Inner(error)) => {
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

enum StepError<E> {
    Inner(E),
    LeaseLost,
    Database(anyhow::Error),
}

async fn with_heartbeat<T, E, F>(
    metadata: &MetadataStore,
    lease: &RequestAttachmentCleanupLease,
    lease_duration: Duration,
    future: F,
) -> Result<T, StepError<E>>
where
    F: Future<Output = Result<T, E>>,
{
    tokio::pin!(future);
    let mut heartbeat = tokio::time::interval(heartbeat_interval(lease_duration));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    loop {
        tokio::select! {
            result = &mut future => return result.map_err(StepError::Inner),
            _ = heartbeat.tick() => {
                let now = crate::unix_now().map_err(StepError::Database)?;
                let renewed = metadata.media().renew_cleanup_lease(
                    &lease.attachment_id,
                    &lease.lease_token,
                    lease.lease_generation,
                    now,
                    lease_expiry(now, lease_duration).map_err(StepError::Database)?,
                ).await.map_err(|error| StepError::Database(db_error(error)))?;
                if !renewed {
                    let _ = future.await;
                    return Err(StepError::LeaseLost);
                }
            }
        }
    }
}

async fn dependencies_ready(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    health: &WorkerHealth,
) -> bool {
    let schema = metadata.admin().readiness_check().await;
    let object_store = storage.readiness_check().await;
    if schema.is_ok() && object_store.is_ok() {
        health.mark_dependencies_ready();
        return true;
    }
    health.mark_dependencies_waiting();
    if let Err(error) = schema {
        tracing::warn!(error = %error.message, "media worker schema fence is unavailable");
    }
    if let Err(error) = object_store {
        tracing::warn!(%error, "media worker object storage is unavailable");
    }
    false
}

fn random_id(prefix: &str) -> anyhow::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("secure random generation failed: {error}"))?;
    Ok(format!("{prefix}-{}", hex::encode(bytes)))
}

fn lease_expiry(now_unix: u64, duration: Duration) -> anyhow::Result<u64> {
    now_unix
        .checked_add(duration.as_secs())
        .ok_or_else(|| anyhow::anyhow!("media lease expiry overflow"))
}

fn heartbeat_interval(lease_duration: Duration) -> Duration {
    Duration::from_secs((lease_duration.as_secs() / 3).max(1))
}

fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(5_u64.saturating_mul(1_u64 << attempt.min(7)).min(600))
}

fn db_error(error: scope_postgres::error::PostgresError) -> anyhow::Error {
    anyhow::anyhow!(error.message)
}

async fn wait_or_shutdown(duration: Duration) -> bool {
    tokio::select! {
        _ = crate::shutdown_signal() => true,
        _ = tokio::time::sleep(duration) => false,
    }
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
