use crate::requests::github_retry_at;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubWebhookDeliveryFailure {
    RateLimited { reset_at_unix: u64 },
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubWebhookDeliveryFinish {
    Done,
    RetryAt(u64),
    RetryUncountedAt(u64),
    GaveUp,
}

impl GitHubWebhookDeliveryFinish {
    pub fn after_failure(
        attempts: u32,
        failure: GitHubWebhookDeliveryFailure,
        now_unix: u64,
    ) -> Self {
        match failure {
            GitHubWebhookDeliveryFailure::RateLimited { reset_at_unix } => {
                Self::RetryUncountedAt(reset_at_unix.max(now_unix.saturating_add(1)))
            }
            GitHubWebhookDeliveryFailure::Failed => {
                github_retry_at(attempts, now_unix).map_or(Self::GaveUp, Self::RetryAt)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_deliveries_back_off_and_give_up_unless_github_is_rate_limiting() {
        let failed = |attempts| {
            GitHubWebhookDeliveryFinish::after_failure(
                attempts,
                GitHubWebhookDeliveryFailure::Failed,
                100,
            )
        };
        assert_eq!(failed(1), GitHubWebhookDeliveryFinish::RetryAt(130));
        assert_eq!(failed(4), GitHubWebhookDeliveryFinish::RetryAt(1900));
        assert_eq!(failed(5), GitHubWebhookDeliveryFinish::GaveUp);
        for (reset_at_unix, retry_at_unix) in [(4_000, 4_000), (50, 101)] {
            assert_eq!(
                GitHubWebhookDeliveryFinish::after_failure(
                    5,
                    GitHubWebhookDeliveryFailure::RateLimited { reset_at_unix },
                    100,
                ),
                GitHubWebhookDeliveryFinish::RetryUncountedAt(retry_at_unix)
            );
        }
    }
}
