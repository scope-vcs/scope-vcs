use scope_service_runtime::readiness::ReadinessTracker;
use std::time::Duration;

/// The four loops every worker runs; readiness needs a recent poll from each.
#[derive(Clone, Copy, Debug)]
pub(crate) enum WorkerLoop {
    Control,
    Compaction,
    Cleanup,
    Dependencies,
}

/// The one startup gate: no loop can do useful work before the schema it
/// expects is present.
const SCHEMA_GATE: usize = 0;

#[derive(Clone)]
pub(crate) struct WorkerHealth(ReadinessTracker<1, 4>);

impl WorkerHealth {
    pub(crate) fn new(poll_interval: Duration) -> Self {
        Self(ReadinessTracker::new(poll_interval))
    }

    pub(crate) fn mark_schema_ready(&self) {
        self.0.open_gate(SCHEMA_GATE);
    }

    pub(crate) fn mark_schema_waiting(&self) {
        self.0.close_gate(SCHEMA_GATE);
    }

    pub(crate) fn mark_poll_succeeded(&self, worker_loop: WorkerLoop, now_unix: u64) {
        self.0.mark_poll(worker_loop as usize, now_unix);
    }

    /// A successful durable lease claim/renewal proves the bounded operation is
    /// being supervised even when it has not reached the next idle poll yet.
    pub(crate) fn mark_work_progress(
        &self,
        worker_loop: WorkerLoop,
        now_unix: u64,
        valid_for: Duration,
    ) {
        self.0
            .mark_progress(worker_loop as usize, now_unix, valid_for);
    }

    pub(crate) async fn serve(self, port: u16) -> anyhow::Result<()> {
        self.0.serve(port, "worker health server").await
    }

    #[cfg(test)]
    fn readyz_at(&self, now_unix: u64) -> axum::http::StatusCode {
        self.0.status_at(now_unix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    const READY: StatusCode = StatusCode::OK;
    const STALE: StatusCode = StatusCode::SERVICE_UNAVAILABLE;

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
        assert_eq!(health.readyz_at(100), STALE);

        health.mark_poll_succeeded(WorkerLoop::Cleanup, 100);
        assert_eq!(health.readyz_at(100), STALE);
        health.mark_poll_succeeded(WorkerLoop::Dependencies, 100);
        assert_eq!(health.readyz_at(115), READY);
        assert_eq!(health.readyz_at(116), STALE);

        health.mark_schema_waiting();
        assert_eq!(health.readyz_at(100), STALE);
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
        assert_eq!(health.readyz_at(115), READY);
        assert_eq!(health.readyz_at(116), STALE);
        health.mark_poll_succeeded(WorkerLoop::Control, 116);
        assert_eq!(health.readyz_at(116), STALE);
    }

    #[test]
    fn active_compaction_is_ready_until_its_lease_progress_expires() {
        let health = WorkerHealth::new(Duration::from_secs(1));
        health.mark_schema_ready();
        health.mark_work_progress(WorkerLoop::Compaction, 100, Duration::from_secs(150));

        refresh_non_compaction_loops(&health, 116);
        assert_eq!(health.readyz_at(116), READY);
        refresh_non_compaction_loops(&health, 200);
        assert_eq!(health.readyz_at(200), READY);
        refresh_non_compaction_loops(&health, 251);
        assert_eq!(health.readyz_at(251), STALE);

        health.mark_work_progress(WorkerLoop::Compaction, 200, Duration::from_secs(150));
        refresh_non_compaction_loops(&health, 251);
        assert_eq!(health.readyz_at(251), READY);
    }
}
