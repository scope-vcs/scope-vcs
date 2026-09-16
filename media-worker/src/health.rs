use scope_service_runtime::readiness::{ActivityGuard, ReadinessTracker};
use std::time::Duration;

/// The two loops the media worker runs; readiness needs a recent poll from each.
#[derive(Clone, Copy, Debug)]
enum MediaWorkerLoop {
    Processing,
    Cleanup,
}

/// Startup gates: decoded codecs, then the database schema and object store.
const CODECS_GATE: usize = 0;
const DEPENDENCIES_GATE: usize = 1;

#[derive(Clone)]
pub struct WorkerHealth(ReadinessTracker<2, 2>);

impl WorkerHealth {
    pub fn new(poll_interval: Duration) -> Self {
        Self(ReadinessTracker::new(poll_interval))
    }

    pub fn mark_codecs_ready(&self) {
        self.0.open_gate(CODECS_GATE);
    }

    pub fn mark_dependencies_ready(&self) {
        self.0.open_gate(DEPENDENCIES_GATE);
    }

    pub fn mark_dependencies_waiting(&self) {
        self.0.close_gate(DEPENDENCIES_GATE);
    }

    pub fn mark_processing_poll(&self, now_unix: u64) {
        self.0
            .mark_poll(MediaWorkerLoop::Processing as usize, now_unix);
    }

    pub fn mark_cleanup_poll(&self, now_unix: u64) {
        self.0
            .mark_poll(MediaWorkerLoop::Cleanup as usize, now_unix);
    }

    /// Conversions and cleanup sweeps outrun their poll interval; the guard
    /// keeps the loop fresh for exactly as long as the work runs.
    pub fn processing_activity(&self) -> ActivityGuard<'_> {
        self.0.activity(MediaWorkerLoop::Processing as usize)
    }

    pub fn cleanup_activity(&self) -> ActivityGuard<'_> {
        self.0.activity(MediaWorkerLoop::Cleanup as usize)
    }

    pub async fn serve(self, port: u16) -> anyhow::Result<()> {
        self.0.serve(port, "media worker health server").await
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

    #[test]
    fn readiness_requires_codecs_dependencies_and_both_recent_loops() {
        let health = WorkerHealth::new(Duration::from_secs(1));
        health.mark_codecs_ready();
        health.mark_dependencies_ready();
        health.mark_processing_poll(100);
        assert_eq!(health.readyz_at(100), STALE);
        health.mark_cleanup_poll(100);
        assert_eq!(health.readyz_at(115), READY);
        assert_eq!(health.readyz_at(116), STALE);
        health.mark_dependencies_waiting();
        assert_eq!(health.readyz_at(100), STALE);
    }

    #[test]
    fn active_long_running_work_keeps_each_loop_fresh() {
        let health = WorkerHealth::new(Duration::from_secs(1));
        health.mark_codecs_ready();
        health.mark_dependencies_ready();
        let processing = health.processing_activity();
        let cleanup = health.cleanup_activity();
        assert_eq!(health.readyz_at(10_000), READY);

        drop(processing);
        assert_eq!(health.readyz_at(10_000), STALE);
        health.mark_processing_poll(10_000);
        assert_eq!(health.readyz_at(10_000), READY);

        drop(cleanup);
        assert_eq!(health.readyz_at(10_016), STALE);
    }
}
