use scope_domain::requests::attachments::RequestAttachmentProcessingLane;
use scope_service_runtime::readiness::{ActivityGuard, ReadinessTracker};
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub enum MediaWorkerLoop {
    PhotoProcessing,
    GeneralProcessing,
    Cleanup,
}

impl From<RequestAttachmentProcessingLane> for MediaWorkerLoop {
    fn from(lane: RequestAttachmentProcessingLane) -> Self {
        match lane {
            RequestAttachmentProcessingLane::Photo => Self::PhotoProcessing,
            RequestAttachmentProcessingLane::General => Self::GeneralProcessing,
        }
    }
}

const CODECS_GATE: usize = 0;
const DEPENDENCIES_GATE: usize = 1;

#[derive(Clone)]
pub struct WorkerHealth(ReadinessTracker<2, 3>);

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

    pub fn mark_poll(&self, worker_loop: MediaWorkerLoop, now_unix: u64) {
        self.0.mark_poll(worker_loop as usize, now_unix);
    }

    pub fn activity(&self, worker_loop: MediaWorkerLoop) -> ActivityGuard<'_> {
        self.0.activity(worker_loop as usize)
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

    const LOOPS: [MediaWorkerLoop; 3] = [
        MediaWorkerLoop::PhotoProcessing,
        MediaWorkerLoop::GeneralProcessing,
        MediaWorkerLoop::Cleanup,
    ];

    #[test]
    fn readiness_requires_codecs_dependencies_and_every_recent_loop() {
        let health = WorkerHealth::new(Duration::from_secs(1));
        health.mark_codecs_ready();
        health.mark_dependencies_ready();
        for (index, worker_loop) in LOOPS.into_iter().enumerate() {
            assert_eq!(health.readyz_at(100), STALE, "{index} loops polled");
            health.mark_poll(worker_loop, 100);
        }
        assert_eq!(health.readyz_at(115), READY);
        assert_eq!(health.readyz_at(116), STALE);
        health.mark_dependencies_waiting();
        assert_eq!(health.readyz_at(100), STALE);
    }
}
