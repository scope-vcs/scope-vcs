use crate::health::{MediaWorkerLoop, WorkerHealth};
use scope_media_storage::MediaStorage;
use scope_postgres::db::MetadataStore;
use scope_service_runtime::{unix_now, wait_or_shutdown};
use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const DEPENDENCY_CHECK_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JobOutcome {
    NoJob,
    Completed,
    Failed,
    LeaseLost,
}

pub(crate) struct JobLoop<'a> {
    pub(crate) worker_loop: MediaWorkerLoop,
    pub(crate) metadata: &'a MetadataStore,
    pub(crate) storage: &'a MediaStorage,
    pub(crate) health: &'a WorkerHealth,
    pub(crate) poll_interval: Duration,
}

impl JobLoop<'_> {
    pub(crate) async fn run<F, Fut>(self, mut next_job: F) -> anyhow::Result<()>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = anyhow::Result<JobOutcome>>,
    {
        let dependency_check = Cadence::new(DEPENDENCY_CHECK_INTERVAL);
        loop {
            let now = unix_now()?;
            let ready = if dependency_check.due(now) {
                let ready = dependencies_ready(self.metadata, self.storage, self.health).await;
                if ready {
                    dependency_check.completed(now);
                }
                ready
            } else {
                true
            };
            if ready {
                let outcome = next_job().await;
                match &outcome {
                    Ok(JobOutcome::NoJob) => {}
                    Ok(outcome) => {
                        tracing::info!(worker_loop = ?self.worker_loop, ?outcome, "media job finished")
                    }
                    Err(error) => {
                        dependency_check.reset();
                        tracing::error!(worker_loop = ?self.worker_loop, %error, "media poll failed");
                    }
                }
                self.health.mark_poll(self.worker_loop, unix_now()?);
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

pub(crate) struct Cadence {
    interval: Duration,
    next_due_unix: AtomicU64,
}

impl Cadence {
    pub(crate) fn new(interval: Duration) -> Self {
        Self {
            interval,
            next_due_unix: AtomicU64::new(0),
        }
    }

    pub(crate) fn due(&self, now_unix: u64) -> bool {
        now_unix >= self.next_due_unix.load(Ordering::Acquire)
    }

    pub(crate) fn completed(&self, now_unix: u64) {
        self.next_due_unix.store(
            now_unix.saturating_add(self.interval.as_secs()),
            Ordering::Release,
        );
    }

    fn reset(&self) {
        self.next_due_unix.store(0, Ordering::Release);
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
