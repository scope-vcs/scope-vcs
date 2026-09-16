//! Readiness for background workers: a worker is ready when every startup gate
//! has opened and every supervised loop has a heartbeat that is not stale.
use axum::{Router, extract::State, http::StatusCode, routing::get};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

/// Four poll intervals before a loop is called stale, never under fifteen
/// seconds: an orchestrator should restart a wedged worker, not a slow one.
const STALE_POLL_INTERVALS: u32 = 4;
const MINIMUM_STALE_AFTER: Duration = Duration::from_secs(15);

/// Tracks `GATES` startup conditions and `LOOPS` supervised loops.
///
/// Workers name their own gates and loops and pass them as indices; this owns
/// when a heartbeat expires and how readiness is served.
#[derive(Clone, Debug)]
pub struct ReadinessTracker<const GATES: usize, const LOOPS: usize> {
    state: Arc<TrackerState<GATES, LOOPS>>,
}

#[derive(Debug)]
struct TrackerState<const GATES: usize, const LOOPS: usize> {
    gates_open: [AtomicBool; GATES],
    /// Unix second through which a loop counts as alive; zero means it has
    /// never reported.
    valid_until_unix: [AtomicU64; LOOPS],
    /// Set while a loop is inside a bounded unit of work that outlives a poll.
    active: [AtomicBool; LOOPS],
    stale_after_secs: u64,
}

impl<const GATES: usize, const LOOPS: usize> ReadinessTracker<GATES, LOOPS> {
    pub fn new(poll_interval: Duration) -> Self {
        let stale_after = poll_interval
            .saturating_mul(STALE_POLL_INTERVALS)
            .max(MINIMUM_STALE_AFTER);
        Self {
            state: Arc::new(TrackerState {
                gates_open: std::array::from_fn(|_| AtomicBool::new(false)),
                valid_until_unix: std::array::from_fn(|_| AtomicU64::new(0)),
                active: std::array::from_fn(|_| AtomicBool::new(false)),
                stale_after_secs: stale_after.as_secs(),
            }),
        }
    }

    pub fn open_gate(&self, gate: usize) {
        self.state.gates_open[gate].store(true, Ordering::Release);
    }

    pub fn close_gate(&self, gate: usize) {
        self.state.gates_open[gate].store(false, Ordering::Release);
    }

    /// A completed poll proves the loop is running and resets its staleness.
    pub fn mark_poll(&self, supervised_loop: usize, now_unix: u64) {
        self.state.valid_until_unix[supervised_loop].store(
            now_unix.saturating_add(self.state.stale_after_secs),
            Ordering::Release,
        );
    }

    /// A durable lease claim or renewal proves a bounded operation is still
    /// supervised even though it has not reached the next idle poll.
    pub fn mark_progress(&self, supervised_loop: usize, now_unix: u64, valid_for: Duration) {
        self.state.valid_until_unix[supervised_loop].fetch_max(
            now_unix.saturating_add(valid_for.as_secs()),
            Ordering::Release,
        );
    }

    /// Holds a loop fresh for work whose duration is not known in advance; the
    /// guard clears the flag even when the work unwinds.
    pub fn activity(&self, supervised_loop: usize) -> ActivityGuard<'_> {
        ActivityGuard::start(&self.state.active[supervised_loop])
    }

    /// The answer `/readyz` gives at `now_unix`.
    pub fn status_at(&self, now_unix: u64) -> StatusCode {
        if self.is_ready_at(now_unix) {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        }
    }

    fn is_ready_at(&self, now_unix: u64) -> bool {
        self.state
            .gates_open
            .iter()
            .all(|gate| gate.load(Ordering::Acquire))
            && (0..LOOPS).all(|index| self.loop_is_alive(index, now_unix))
    }

    /// Serves `/readyz` until shutdown; a stale or ungated worker answers 503.
    pub async fn serve(self, port: u16, label: &'static str) -> anyhow::Result<()> {
        let app = Router::new().route("/readyz", get(readyz)).with_state(self);
        crate::serve(port, app, label).await
    }

    fn loop_is_alive(&self, supervised_loop: usize, now_unix: u64) -> bool {
        if self.state.active[supervised_loop].load(Ordering::Acquire) {
            return true;
        }
        let valid_until = self.state.valid_until_unix[supervised_loop].load(Ordering::Acquire);
        valid_until > 0 && now_unix <= valid_until
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

async fn readyz<const GATES: usize, const LOOPS: usize>(
    State(tracker): State<ReadinessTracker<GATES, LOOPS>>,
) -> StatusCode {
    match crate::unix_now() {
        Ok(now_unix) => tracker.status_at(now_unix),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READY: StatusCode = StatusCode::OK;
    const STALE: StatusCode = StatusCode::SERVICE_UNAVAILABLE;

    #[test]
    fn a_short_poll_interval_still_tolerates_fifteen_seconds_of_silence() {
        let tracker = ReadinessTracker::<1, 1>::new(Duration::from_secs(1));
        tracker.open_gate(0);
        tracker.mark_poll(0, 100);

        assert_eq!(tracker.status_at(115), READY);
        assert_eq!(tracker.status_at(116), STALE);
    }

    #[test]
    fn a_long_poll_interval_scales_the_staleness_window() {
        let tracker = ReadinessTracker::<1, 1>::new(Duration::from_secs(30));
        tracker.open_gate(0);
        tracker.mark_poll(0, 100);

        assert_eq!(tracker.status_at(220), READY);
        assert_eq!(tracker.status_at(221), STALE);
    }

    #[test]
    fn readiness_needs_every_gate_and_every_loop() {
        let tracker = ReadinessTracker::<2, 2>::new(Duration::from_secs(1));
        tracker.open_gate(0);
        tracker.mark_poll(0, 100);
        tracker.mark_poll(1, 100);
        assert_eq!(tracker.status_at(100), STALE);

        tracker.open_gate(1);
        assert_eq!(tracker.status_at(100), READY);

        tracker.close_gate(1);
        assert_eq!(tracker.status_at(100), STALE);
        tracker.open_gate(1);

        // One silent loop is enough to fail readiness.
        tracker.mark_poll(0, 110);
        assert_eq!(tracker.status_at(116), STALE);
    }

    #[test]
    fn leased_progress_and_open_ended_activity_both_hold_a_loop_alive() {
        let tracker = ReadinessTracker::<1, 2>::new(Duration::from_secs(1));
        tracker.open_gate(0);
        tracker.mark_poll(0, 100);
        tracker.mark_progress(1, 100, Duration::from_secs(150));

        assert_eq!(tracker.status_at(115), READY);
        assert_eq!(tracker.status_at(116), STALE);

        tracker.mark_poll(0, 200);
        assert_eq!(tracker.status_at(200), READY);
        assert_eq!(tracker.status_at(251), STALE);

        let activity = tracker.activity(1);
        tracker.mark_poll(0, 251);
        assert_eq!(tracker.status_at(251), READY);
        drop(activity);
        assert_eq!(tracker.status_at(251), STALE);
    }

    #[tokio::test]
    async fn the_served_route_answers_from_the_wall_clock() {
        let tracker = ReadinessTracker::<1, 1>::new(Duration::from_secs(1));
        tracker.open_gate(0);
        let now_unix = crate::unix_now().unwrap();

        tracker.mark_poll(0, now_unix.saturating_sub(1_000));
        assert_eq!(readyz(State(tracker.clone())).await, STALE);

        tracker.mark_poll(0, now_unix);
        assert_eq!(readyz(State(tracker)).await, READY);
    }
}
