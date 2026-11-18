use super::client::RATE_LIMIT_MESSAGE;
use crate::error::ApiError;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

pub(crate) type UnixClock = Arc<dyn Fn() -> Result<u64, ApiError> + Send + Sync>;

pub(super) struct RateLimitPauses {
    now_unix: UnixClock,
    until_unix: Mutex<HashMap<u64, u64>>,
}

impl RateLimitPauses {
    pub(super) fn new(now_unix: UnixClock) -> Self {
        Self {
            now_unix,
            until_unix: Mutex::default(),
        }
    }

    pub(super) fn check(&self, installation_id: u64) -> Result<(), ApiError> {
        let now_unix = (self.now_unix)()?;
        let mut until_unix = self
            .until_unix
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match until_unix.get(&installation_id).copied() {
            Some(until) if now_unix < until => {
                Err(ApiError::upstream_rate_limited(RATE_LIMIT_MESSAGE, until))
            }
            Some(_) => {
                until_unix.remove(&installation_id);
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub(super) fn note<T>(
        &self,
        installation_id: u64,
        answer: Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        if let Err(error) = &answer
            && let Some(reported) = error.retry_at_unix()
        {
            self.until_unix
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(installation_id)
                .and_modify(|until| *until = reported.max(*until))
                .or_insert(reported);
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn a_reported_rate_limit_pauses_that_installation_until_its_longest_reset() {
        let now = Arc::new(AtomicU64::new(499));
        let pauses = RateLimitPauses::new(Arc::new({
            let now = now.clone();
            move || Ok(now.load(Ordering::SeqCst))
        }));
        for reported in [500, 440] {
            let refused: Result<(), _> = Err(ApiError::upstream_rate_limited("limited", reported));
            assert!(pauses.note(7, refused).is_err());
        }
        assert!(
            pauses
                .note(7, Err::<(), _>(ApiError::not_found("gone")))
                .is_err()
        );

        assert_eq!(pauses.check(7).unwrap_err().retry_at_unix(), Some(500));
        pauses.check(8).unwrap();
        now.store(500, Ordering::SeqCst);
        pauses.check(7).unwrap();
        now.store(499, Ordering::SeqCst);
        pauses.check(7).unwrap();
    }
}
