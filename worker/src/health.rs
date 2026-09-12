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
    last_successful_poll_unix: [AtomicU64; 4],
    stale_after_secs: u64,
}

impl WorkerHealth {
    pub(crate) fn new(poll_interval: Duration) -> Self {
        let stale_after_secs = poll_interval.as_secs().saturating_mul(3).max(10);
        Self {
            state: Arc::new(WorkerHealthState {
                schema_ready: AtomicBool::new(false),
                last_successful_poll_unix: std::array::from_fn(|_| AtomicU64::new(0)),
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
        self.state.last_successful_poll_unix[worker_loop as usize]
            .store(now_unix, Ordering::Release);
    }

    pub(crate) async fn serve(self, port: u16) -> anyhow::Result<()> {
        let app = Router::new().route("/readyz", get(readyz)).with_state(self);
        scope_service_runtime::serve(port, app, "worker health server").await
    }

    fn is_ready_at(&self, now_unix: u64) -> bool {
        self.state.schema_ready.load(Ordering::Acquire)
            && self
                .state
                .last_successful_poll_unix
                .iter()
                .all(|last_success| {
                    let last_success = last_success.load(Ordering::Acquire);
                    last_success > 0
                        && now_unix.saturating_sub(last_success) <= self.state.stale_after_secs
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
}
