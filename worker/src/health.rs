use crate::settings::WorkerRole;
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
pub(crate) struct WorkerHealth {
    state: Arc<WorkerHealthState>,
}

struct WorkerHealthState {
    schema_ready: AtomicBool,
    valid_until_unix: [AtomicU64; 3],
    required_roles: [bool; 3],
    stale_after_secs: u64,
}

impl WorkerHealth {
    pub(crate) fn new(poll_interval: Duration, role: WorkerRole) -> Self {
        let stale_after_secs = poll_interval.as_secs().saturating_mul(3).max(10);
        Self {
            state: Arc::new(WorkerHealthState {
                schema_ready: AtomicBool::new(false),
                valid_until_unix: std::array::from_fn(|_| AtomicU64::new(0)),
                required_roles: [
                    role.runs_control(),
                    role.runs_compaction(),
                    role.runs_cleanup(),
                ],
                stale_after_secs,
            }),
        }
    }

    pub(crate) fn mark_schema_waiting(&self) {
        self.state.schema_ready.store(false, Ordering::Release);
    }

    pub(crate) fn mark_poll_succeeded(&self, role: WorkerRole, now_unix: u64) {
        self.state.valid_until_unix[concrete_role_index(role)].store(
            now_unix.saturating_add(self.state.stale_after_secs),
            Ordering::Release,
        );
        self.state.schema_ready.store(true, Ordering::Release);
    }

    /// A successful durable lease claim/renewal proves the bounded operation is
    /// being supervised even when it has not reached the next idle poll yet.
    pub(crate) fn mark_work_progress(&self, role: WorkerRole, now_unix: u64, valid_for: Duration) {
        self.state.valid_until_unix[concrete_role_index(role)].fetch_max(
            now_unix.saturating_add(valid_for.as_secs()),
            Ordering::Release,
        );
        self.state.schema_ready.store(true, Ordering::Release);
    }

    pub(crate) async fn serve(self, port: u16) -> anyhow::Result<()> {
        let addr = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
        let app = Router::new()
            .route("/healthz", get(healthz))
            .with_state(self);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!(%addr, "starting worker health server");
        axum::serve(listener, app)
            .with_graceful_shutdown(super::shutdown_signal())
            .await?;
        Ok(())
    }

    fn is_ready_at(&self, now_unix: u64) -> bool {
        if !self.state.schema_ready.load(Ordering::Acquire) {
            return false;
        }
        self.state
            .required_roles
            .iter()
            .enumerate()
            .all(|(index, required)| {
                if !required {
                    return true;
                }
                let valid_until = self.state.valid_until_unix[index].load(Ordering::Acquire);
                valid_until > 0 && now_unix <= valid_until
            })
    }
}

fn concrete_role_index(role: WorkerRole) -> usize {
    match role {
        WorkerRole::Control => 0,
        WorkerRole::Compaction => 1,
        WorkerRole::Cleanup => 2,
        WorkerRole::All => panic!("health updates require one concrete worker role"),
    }
}

async fn healthz(State(health): State<WorkerHealth>) -> StatusCode {
    match super::unix_now() {
        Ok(now_unix) if health.is_ready_at(now_unix) => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_requires_matching_schema_and_a_recent_poll() {
        let health = WorkerHealth::new(Duration::from_secs(1), WorkerRole::Control);
        assert!(!health.is_ready_at(100));

        health.mark_poll_succeeded(WorkerRole::Control, 100);
        assert!(health.is_ready_at(110));
        assert!(!health.is_ready_at(111));

        health.mark_schema_waiting();
        assert!(!health.is_ready_at(100));
    }

    #[test]
    fn active_compaction_is_ready_until_its_lease_progress_expires() {
        let health = WorkerHealth::new(Duration::from_secs(1), WorkerRole::Compaction);
        health.mark_work_progress(WorkerRole::Compaction, 100, Duration::from_secs(150));
        assert!(health.is_ready_at(111));
        assert!(health.is_ready_at(200));
        assert!(!health.is_ready_at(251));
        health.mark_work_progress(WorkerRole::Compaction, 200, Duration::from_secs(150));
        assert!(health.is_ready_at(251));
        health.mark_poll_succeeded(WorkerRole::Compaction, 260);
        assert!(health.is_ready_at(270));
        assert!(!health.is_ready_at(271));
        let slow_poll = WorkerHealth::new(Duration::from_secs(100), WorkerRole::Compaction);
        slow_poll.mark_poll_succeeded(WorkerRole::Compaction, 100);
        slow_poll.mark_work_progress(WorkerRole::Compaction, 110, Duration::from_secs(150));
        assert!(slow_poll.is_ready_at(400));
        assert!(!slow_poll.is_ready_at(401));
    }

    #[test]
    fn all_role_health_requires_every_loop() {
        let health = WorkerHealth::new(Duration::from_secs(1), WorkerRole::All);
        health.mark_poll_succeeded(WorkerRole::Control, 100);
        health.mark_poll_succeeded(WorkerRole::Compaction, 100);
        assert!(!health.is_ready_at(100));

        health.mark_poll_succeeded(WorkerRole::Cleanup, 100);
        assert!(health.is_ready_at(100));
    }
}
