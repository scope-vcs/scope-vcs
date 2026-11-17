use super::client::RATE_LIMIT_MESSAGE;
use crate::error::ApiError;
use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

#[derive(Default)]
pub(super) struct RateLimitPauses {
    until_unix: Mutex<HashMap<u64, u64>>,
}

impl RateLimitPauses {
    pub(super) fn check(&self, installation_id: u64, now_unix: u64) -> Result<(), ApiError> {
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
            && let Some(until) = error.retry_at_unix()
        {
            self.until_unix
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(installation_id, until);
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reported_rate_limit_pauses_that_installation_until_its_reset() {
        let pauses = RateLimitPauses::default();
        let refused: Result<(), _> = Err(ApiError::upstream_rate_limited("limited", 500));
        assert!(pauses.note(7, refused).is_err());
        assert!(
            pauses
                .note(7, Err::<(), _>(ApiError::not_found("gone")))
                .is_err()
        );

        assert_eq!(pauses.check(7, 499).unwrap_err().retry_at_unix(), Some(500));
        pauses.check(8, 499).unwrap();
        pauses.check(7, 500).unwrap();
        pauses.check(7, 499).unwrap();
    }
}
