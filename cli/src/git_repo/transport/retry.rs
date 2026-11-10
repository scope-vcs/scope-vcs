use crate::error::{CliError, GitRetrySafety};
use std::time::Duration;

const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(4),
    Duration::from_secs(10),
];
const IMMEDIATE_RETRY_DELAYS: [Duration; 3] = [Duration::ZERO; 3];

pub(super) fn with_git_retries<T>(
    retry_safety: GitRetrySafety,
    operation: impl FnMut() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let delays = if cfg!(test) {
        &IMMEDIATE_RETRY_DELAYS
    } else {
        &RETRY_DELAYS
    };
    retry_with_delays(
        delays,
        retry_safety,
        std::thread::sleep,
        announce_retry,
        operation,
    )
}

fn announce_retry(delay: Duration, next_attempt: usize, attempts: usize) {
    if !crate::execution::json() {
        eprintln!(
            "Scope Git remote is temporarily unavailable; retrying in {}s (attempt {next_attempt} of {attempts})",
            delay.as_secs()
        );
    }
}

fn retry_with_delays<T>(
    delays: &[Duration],
    retry_safety: GitRetrySafety,
    mut wait: impl FnMut(Duration),
    mut announce: impl FnMut(Duration, usize, usize),
    mut operation: impl FnMut() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let attempts = delays.len() + 1;
    let mut delays = delays.iter();
    loop {
        let error = match operation() {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let Some(&delay) = delays.next() else {
            return Err(error);
        };
        if !may_retry(retry_safety, &error) {
            return Err(error);
        }
        let next_attempt = attempts - delays.len();
        announce(delay, next_attempt, attempts);
        wait(delay);
    }
}

fn may_retry(retry_safety: GitRetrySafety, error: &anyhow::Error) -> bool {
    matches!(retry_safety, GitRetrySafety::Idempotent)
        && error
            .downcast_ref::<CliError>()
            .is_some_and(|error| error.response().retryable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_api_contract::{ErrorCode, ErrorResponse};

    fn unavailable() -> anyhow::Error {
        CliError::git_failure(
            "git fetch failed",
            "fatal: unable to access: The requested URL returned error: 503",
            GitRetrySafety::Idempotent,
        )
        .into()
    }

    fn rejected() -> anyhow::Error {
        CliError::new(ErrorResponse::new(ErrorCode::Forbidden, "denied")).into()
    }

    struct Run {
        calls: usize,
        waits: Vec<Duration>,
        announcements: usize,
        result: anyhow::Result<usize>,
    }

    fn run(
        retry_safety: GitRetrySafety,
        mut outcomes: Vec<anyhow::Result<()>>,
        delays: &[Duration],
    ) -> Run {
        outcomes.reverse();
        let mut calls = 0;
        let mut waits = Vec::new();
        let mut announcements = 0;
        let result = retry_with_delays(
            delays,
            retry_safety,
            |delay| waits.push(delay),
            |_, _, _| announcements += 1,
            || {
                calls += 1;
                outcomes.pop().unwrap().map(|()| calls)
            },
        );
        Run {
            calls,
            waits,
            announcements,
            result,
        }
    }

    #[test]
    fn a_temporary_failure_is_retried_until_it_succeeds() {
        let run = run(
            GitRetrySafety::Idempotent,
            vec![Err(unavailable()), Err(unavailable()), Ok(())],
            &RETRY_DELAYS,
        );
        assert_eq!(run.result.unwrap(), 3);
        assert_eq!(run.waits, [RETRY_DELAYS[0], RETRY_DELAYS[1]]);
        assert_eq!(run.announcements, 2);
    }

    #[test]
    fn exhausted_attempts_return_the_last_error() {
        let run = run(
            GitRetrySafety::Idempotent,
            (0..4).map(|_| Err(unavailable())).collect(),
            &RETRY_DELAYS,
        );
        let error = run.result.unwrap_err();
        assert_eq!(run.calls, 4);
        assert_eq!(run.waits, RETRY_DELAYS);
        assert_eq!(
            error.downcast_ref::<CliError>().unwrap().response().code,
            ErrorCode::ServiceUnavailable
        );
    }

    #[test]
    fn a_push_that_may_create_a_request_is_never_retried() {
        let run = run(
            GitRetrySafety::MayCreateRequest,
            vec![Err(unavailable()), Ok(())],
            &RETRY_DELAYS,
        );
        assert!(run.result.is_err());
        assert_eq!(run.calls, 1);
        assert!(run.waits.is_empty());
    }

    #[test]
    fn a_permanent_failure_is_not_retried() {
        let run = run(
            GitRetrySafety::Idempotent,
            vec![Err(rejected()), Ok(())],
            &RETRY_DELAYS,
        );
        assert!(run.result.is_err());
        assert_eq!(run.calls, 1);
        assert!(run.waits.is_empty());
    }
}
