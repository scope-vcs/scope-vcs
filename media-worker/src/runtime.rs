use crate::health::WorkerHealth;
use scope_media_storage::MediaStorage;
use scope_postgres::db::MetadataStore;
use scope_service_runtime::{unix_now, wait_or_shutdown};
use std::{future::Future, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JobOutcome {
    NoJob,
    Completed,
    Failed,
    LeaseLost,
}

pub(crate) struct JobLoop<'a> {
    pub(crate) kind: &'static str,
    pub(crate) metadata: &'a MetadataStore,
    pub(crate) storage: &'a MediaStorage,
    pub(crate) health: &'a WorkerHealth,
    pub(crate) poll_interval: Duration,
    pub(crate) mark_poll: fn(&WorkerHealth, u64),
}

impl JobLoop<'_> {
    pub(crate) async fn run<F, Fut>(self, mut next_job: F) -> anyhow::Result<()>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = anyhow::Result<JobOutcome>>,
    {
        let kind = self.kind;
        loop {
            if dependencies_ready(self.metadata, self.storage, self.health).await {
                let outcome = next_job().await;
                match &outcome {
                    Ok(JobOutcome::NoJob) => {}
                    Ok(outcome) => tracing::info!(?outcome, "media {kind} job finished"),
                    Err(error) => tracing::error!(%error, "media {kind} poll failed"),
                }
                (self.mark_poll)(self.health, unix_now()?);
                if !matches!(outcome, Ok(JobOutcome::NoJob) | Err(_)) {
                    continue;
                }
            }
            if wait_or_shutdown(self.poll_interval).await {
                return Ok(());
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

pub(crate) fn lease_expiry(now_unix: u64, duration: Duration) -> anyhow::Result<u64> {
    now_unix
        .checked_add(duration.as_secs())
        .ok_or_else(|| anyhow::anyhow!("media lease expiry overflow"))
}

pub(crate) fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(5_u64.saturating_mul(1_u64 << attempt.min(7)).min(600))
}
