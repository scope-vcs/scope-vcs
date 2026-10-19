use std::{future::Future, time::Duration};
use tokio::time::Instant;

#[derive(Debug)]
pub(crate) enum HeartbeatError {
    LeaseLost,
    Database(anyhow::Error),
}

struct RenewalSchedule {
    lease_duration: Duration,
    heartbeat: Duration,
    retry_delay: Duration,
    safety_margin: Duration,
}

impl RenewalSchedule {
    fn new(lease_duration: Duration) -> Self {
        Self {
            lease_duration,
            heartbeat: (lease_duration / 3).max(Duration::from_millis(10)),
            retry_delay: lease_duration / 24,
            safety_margin: lease_duration / 4,
        }
    }

    fn retry_at(&self, granted_until: Instant) -> Option<Instant> {
        let retry_at = Instant::now() + self.retry_delay;
        let give_up_at = granted_until.checked_sub(self.safety_margin)?;
        (retry_at < give_up_at).then_some(retry_at)
    }
}

pub(crate) async fn supervise_lease<T, F, R, RFut>(
    future: F,
    lease_duration: Duration,
    mut renew: R,
) -> Result<T, HeartbeatError>
where
    F: Future<Output = T>,
    R: FnMut() -> RFut,
    RFut: Future<Output = anyhow::Result<bool>>,
{
    tokio::pin!(future);
    let schedule = RenewalSchedule::new(lease_duration);
    let mut granted_until = Instant::now() + schedule.lease_duration;
    let mut next_renewal = Instant::now() + schedule.heartbeat;
    loop {
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = tokio::time::sleep_until(next_renewal) => {}
        }
        let requested_at = Instant::now();
        let renewal = renew();
        tokio::pin!(renewal);
        let renewed = tokio::select! {
            result = &mut future => return Ok(result),
            renewed = &mut renewal => renewed,
        };
        match renewed {
            Ok(true) => {
                granted_until = requested_at + schedule.lease_duration;
                next_renewal = Instant::now() + schedule.heartbeat;
            }
            Ok(false) => return Err(HeartbeatError::LeaseLost),
            Err(error) => {
                let Some(retry_at) = schedule.retry_at(granted_until) else {
                    return Err(HeartbeatError::Database(error));
                };
                tracing::warn!(%error, "lease renewal failed; retrying before the lease expires");
                next_renewal = retry_at;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone)]
    struct LeaseRow {
        lease_duration: Duration,
        expires_at: Arc<Mutex<Instant>>,
        renewals: Arc<AtomicUsize>,
        failures_remaining: Arc<AtomicUsize>,
        failures: Arc<AtomicUsize>,
    }

    impl LeaseRow {
        fn granted(lease_duration: Duration, failing_renewals: usize) -> Self {
            Self {
                lease_duration,
                expires_at: Arc::new(Mutex::new(Instant::now() + lease_duration)),
                renewals: Arc::default(),
                failures_remaining: Arc::new(AtomicUsize::new(failing_renewals)),
                failures: Arc::default(),
            }
        }

        fn renew(&self) -> impl Future<Output = anyhow::Result<bool>> + use<> {
            let row = self.clone();
            async move {
                if row
                    .failures_remaining
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
                        left.checked_sub(1)
                    })
                    .is_ok()
                {
                    row.failures.fetch_add(1, Ordering::Relaxed);
                    anyhow::bail!("database unavailable");
                }
                let now = Instant::now();
                let mut expires_at = row.expires_at.lock().expect("lease expiry lock");
                if now >= *expires_at {
                    return Ok(false);
                }
                *expires_at = now + row.lease_duration;
                row.renewals.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            }
        }
    }

    const LEASE: Duration = Duration::from_secs(120);

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn heartbeat_keeps_a_claim_alive_beyond_its_initial_lease() {
        let row = LeaseRow::granted(LEASE, 0);

        let result = supervise_lease(
            async {
                tokio::time::sleep(LEASE * 4).await;
                42
            },
            LEASE,
            || row.renew(),
        )
        .await
        .expect("claim should remain leased");

        assert_eq!(result, 42);
        assert!(row.renewals.load(Ordering::Relaxed) >= 11);
        assert!(*row.expires_at.lock().expect("lease expiry lock") > Instant::now());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn brief_renewal_failures_are_retried_before_the_lease_expires() {
        let row = LeaseRow::granted(LEASE, 3);

        let result = supervise_lease(
            async {
                tokio::time::sleep(LEASE * 4).await;
                42
            },
            LEASE,
            || row.renew(),
        )
        .await
        .expect("claim should survive a brief database outage");

        assert_eq!(result, 42);
        assert_eq!(row.failures.load(Ordering::Relaxed), 3);
        assert!(*row.expires_at.lock().expect("lease expiry lock") > Instant::now());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn persistent_renewal_failures_release_the_claim_before_the_lease_expires() {
        let row = LeaseRow::granted(LEASE, usize::MAX);
        let started = Instant::now();

        let outcome = supervise_lease(std::future::pending::<()>(), LEASE, || row.renew()).await;

        assert!(matches!(outcome, Err(HeartbeatError::Database(_))));
        assert!(started.elapsed() <= LEASE - LEASE / 4);
        assert!(row.failures.load(Ordering::Relaxed) > 1);
        assert_eq!(row.renewals.load(Ordering::Relaxed), 0);
    }
}
