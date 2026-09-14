use axum::{Router, extract::State, http::StatusCode, routing::get};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

/// The four loops every worker runs; readiness needs a recent poll from each.
#[derive(Clone, Copy, Debug)]
pub(crate) enum WorkerLoop {
    Control,
    Compaction,
    Cleanup,
    Dependencies,
}

#[derive(Clone)]
pub(crate) struct WorkerHealth {
    state: Arc<WorkerHealthState>,
}

struct WorkerHealthState {
    schema_ready: AtomicBool,
    valid_until_unix: [AtomicU64; 4],
    stale_after_secs: u64,
}

impl WorkerHealth {
    pub(crate) fn new(poll_interval: Duration) -> Self {
        let stale_after_secs = poll_interval.as_secs().saturating_mul(3).max(10);
        Self {
            state: Arc::new(WorkerHealthState {
                schema_ready: AtomicBool::new(false),
                valid_until_unix: std::array::from_fn(|_| AtomicU64::new(0)),
                stale_after_secs,
            }),
        }
    }

    pub(crate) fn mark_schema_ready(&self) {
        self.state.schema_ready.store(true, Ordering::Release);
    }

    pub(crate) fn mark_schema_waiting(&self) {
        self.state.schema_ready.store(false, Ordering::Release);
    }

    pub(crate) fn mark_poll_succeeded(&self, worker_loop: WorkerLoop, now_unix: u64) {
        self.state.valid_until_unix[worker_loop as usize].store(
            now_unix.saturating_add(self.state.stale_after_secs),
            Ordering::Release,
        );
    }

    /// A successful durable lease claim/renewal proves the bounded operation is
    /// being supervised even when it has not reached the next idle poll yet.
    pub(crate) fn mark_work_progress(
        &self,
        worker_loop: WorkerLoop,
        now_unix: u64,
        valid_for: Duration,
    ) {
        self.state.valid_until_unix[worker_loop as usize].fetch_max(
            now_unix.saturating_add(valid_for.as_secs()),
            Ordering::Release,
        );
    }

    pub(crate) async fn serve(self, port: u16) -> anyhow::Result<()> {
        let app = Router::new().route("/readyz", get(readyz)).with_state(self);
        scope_service_runtime::serve(port, app, "worker health server").await
    }

    fn is_ready_at(&self, now_unix: u64) -> bool {
        self.state.schema_ready.load(Ordering::Acquire)
            && self.state.valid_until_unix.iter().all(|valid_until| {
                let valid_until = valid_until.load(Ordering::Acquire);
                valid_until > 0 && now_unix <= valid_until
            })
    }
}

async fn readyz(State(health): State<WorkerHealth>) -> StatusCode {
    match super::unix_now() {
        Ok(now_unix) if health.is_ready_at(now_unix) => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refresh_non_compaction_loops(health: &WorkerHealth, now_unix: u64) {
        for worker_loop in [
            WorkerLoop::Control,
            WorkerLoop::Cleanup,
            WorkerLoop::Dependencies,
        ] {
            health.mark_poll_succeeded(worker_loop, now_unix);
        }
    }

    #[test]
    fn readiness_requires_a_ready_schema_and_a_recent_poll_from_every_loop() {
        let health = WorkerHealth::new(Duration::from_secs(1));
        health.mark_schema_ready();
        health.mark_poll_succeeded(WorkerLoop::Control, 100);
        health.mark_poll_succeeded(WorkerLoop::Compaction, 100);
        assert!(!health.is_ready_at(100));

        health.mark_poll_succeeded(WorkerLoop::Cleanup, 100);
        assert!(!health.is_ready_at(100));
        health.mark_poll_succeeded(WorkerLoop::Dependencies, 100);
        assert!(health.is_ready_at(110));
        assert!(!health.is_ready_at(111));

        health.mark_schema_waiting();
        assert!(!health.is_ready_at(100));
    }

    #[test]
    fn work_failures_do_not_report_the_schema_as_waiting() {
        let health = WorkerHealth::new(Duration::from_secs(1));
        health.mark_schema_ready();
        for worker_loop in [
            WorkerLoop::Control,
            WorkerLoop::Compaction,
            WorkerLoop::Cleanup,
            WorkerLoop::Dependencies,
        ] {
            health.mark_poll_succeeded(worker_loop, 100);
        }
        // A loop that stops polling goes stale on its own timestamp; the schema
        // flag is owned by the readiness check alone.
        assert!(health.is_ready_at(110));
        assert!(!health.is_ready_at(111));
        health.mark_poll_succeeded(WorkerLoop::Control, 111);
        assert!(!health.is_ready_at(111));
    }

    #[test]
    fn active_compaction_is_ready_until_its_lease_progress_expires() {
        let health = WorkerHealth::new(Duration::from_secs(1));
        health.mark_schema_ready();
        health.mark_work_progress(WorkerLoop::Compaction, 100, Duration::from_secs(150));

        refresh_non_compaction_loops(&health, 111);
        assert!(health.is_ready_at(111));
        refresh_non_compaction_loops(&health, 200);
        assert!(health.is_ready_at(200));
        refresh_non_compaction_loops(&health, 251);
        assert!(!health.is_ready_at(251));

        health.mark_work_progress(WorkerLoop::Compaction, 200, Duration::from_secs(150));
        refresh_non_compaction_loops(&health, 251);
        assert!(health.is_ready_at(251));
    }
}
