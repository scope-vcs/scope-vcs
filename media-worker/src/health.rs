use axum::{Router, extract::State, http::StatusCode, routing::get};
use std::{
    net::{Ipv6Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
pub struct WorkerHealth {
    state: Arc<HealthState>,
}

struct HealthState {
    codecs_ready: AtomicBool,
    dependencies_ready: AtomicBool,
    processing_poll_unix: AtomicU64,
    cleanup_poll_unix: AtomicU64,
    processing_active: AtomicBool,
    cleanup_active: AtomicBool,
    stale_after_secs: u64,
}

impl WorkerHealth {
    pub fn new(poll_interval: Duration) -> Self {
        Self {
            state: Arc::new(HealthState {
                codecs_ready: AtomicBool::new(false),
                dependencies_ready: AtomicBool::new(false),
                processing_poll_unix: AtomicU64::new(0),
                cleanup_poll_unix: AtomicU64::new(0),
                processing_active: AtomicBool::new(false),
                cleanup_active: AtomicBool::new(false),
                stale_after_secs: poll_interval.as_secs().saturating_mul(4).max(15),
            }),
        }
    }

    pub fn mark_codecs_ready(&self) {
        self.state.codecs_ready.store(true, Ordering::Release);
    }

    pub fn mark_dependencies_ready(&self) {
        self.state.dependencies_ready.store(true, Ordering::Release);
    }

    pub fn mark_dependencies_waiting(&self) {
        self.state
            .dependencies_ready
            .store(false, Ordering::Release);
    }

    pub fn mark_processing_poll(&self, now_unix: u64) {
        self.state
            .processing_poll_unix
            .store(now_unix, Ordering::Release);
    }

    pub fn mark_cleanup_poll(&self, now_unix: u64) {
        self.state
            .cleanup_poll_unix
            .store(now_unix, Ordering::Release);
    }

    pub fn processing_activity(&self) -> ActivityGuard<'_> {
        ActivityGuard::start(&self.state.processing_active)
    }

    pub fn cleanup_activity(&self) -> ActivityGuard<'_> {
        ActivityGuard::start(&self.state.cleanup_active)
    }

    pub async fn serve(self, port: u16) -> anyhow::Result<()> {
        let address = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
        let router = Router::new()
            .route("/health", get(health))
            .route("/healthz", get(health))
            .with_state(self);
        let listener = tokio::net::TcpListener::bind(address).await?;
        tracing::info!(%address, "media worker health server listening");
        axum::serve(listener, router)
            .with_graceful_shutdown(crate::shutdown_signal())
            .await?;
        Ok(())
    }

    fn ready_at(&self, now_unix: u64) -> bool {
        self.state.codecs_ready.load(Ordering::Acquire)
            && self.state.dependencies_ready.load(Ordering::Acquire)
            && (self.state.processing_active.load(Ordering::Acquire)
                || recent(
                    &self.state.processing_poll_unix,
                    now_unix,
                    self.state.stale_after_secs,
                ))
            && (self.state.cleanup_active.load(Ordering::Acquire)
                || recent(
                    &self.state.cleanup_poll_unix,
                    now_unix,
                    self.state.stale_after_secs,
                ))
    }
}

pub struct ActivityGuard<'a>(&'a AtomicBool);

impl<'a> ActivityGuard<'a> {
    fn start(active: &'a AtomicBool) -> Self {
        active.store(true, Ordering::Release);
        Self(active)
    }
}

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn recent(value: &AtomicU64, now_unix: u64, stale_after_secs: u64) -> bool {
    let value = value.load(Ordering::Acquire);
    value > 0 && now_unix.saturating_sub(value) <= stale_after_secs
}

async fn health(State(health): State<WorkerHealth>) -> StatusCode {
    match crate::unix_now() {
        Ok(now_unix) if health.ready_at(now_unix) => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_requires_codecs_dependencies_and_both_recent_loops() {
        let health = WorkerHealth::new(Duration::from_secs(1));
        health.mark_codecs_ready();
        health.mark_dependencies_ready();
        health.mark_processing_poll(100);
        assert!(!health.ready_at(100));
        health.mark_cleanup_poll(100);
        assert!(health.ready_at(100));
        assert!(!health.ready_at(116));
        health.mark_dependencies_waiting();
        assert!(!health.ready_at(100));
    }

    #[test]
    fn active_long_running_work_keeps_each_loop_fresh() {
        let health = WorkerHealth::new(Duration::from_secs(1));
        health.mark_codecs_ready();
        health.mark_dependencies_ready();
        let processing = health.processing_activity();
        let cleanup = health.cleanup_activity();
        assert!(health.ready_at(10_000));

        drop(processing);
        assert!(!health.ready_at(10_000));
        health.mark_processing_poll(10_000);
        assert!(health.ready_at(10_000));

        drop(cleanup);
        assert!(!health.ready_at(10_016));
    }
}
