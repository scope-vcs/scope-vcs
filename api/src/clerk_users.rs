//! Deletes Clerk users through Clerk's Backend API.

use crate::config::{CLERK_SECRET_KEY_ENV, non_empty_env};
use std::{sync::Arc, time::Duration};

/// The longest one Clerk Backend API call may take.
pub(crate) const CLERK_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

const CLERK_USERS_URL: &str = "https://api.clerk.com/v1/users";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClerkUserDeletion {
    /// Clerk no longer has the user, whether this attempt removed it or an
    /// earlier one did.
    Deleted,
    /// Try again later. Deletion is never abandoned.
    Retry(String),
}

#[derive(Clone)]
pub(crate) enum ClerkUsers {
    Api(Arc<ClerkBackendApi>),
    /// No secret key is configured. Deletions stay queued and are retried,
    /// so they complete once a key is set.
    Unconfigured,
    #[cfg(test)]
    Scripted(Arc<ScriptedClerkUsers>),
}

pub(crate) struct ClerkBackendApi {
    client: reqwest::Client,
    secret_key: String,
}

impl ClerkUsers {
    pub(crate) fn from_env() -> Self {
        let Some(secret_key) = non_empty_env(CLERK_SECRET_KEY_ENV) else {
            tracing::warn!(
                "{CLERK_SECRET_KEY_ENV} is not set; deleted accounts keep their Clerk users until it is"
            );
            return Self::Unconfigured;
        };
        Self::Api(Arc::new(ClerkBackendApi {
            client: reqwest::Client::builder()
                .timeout(CLERK_REQUEST_TIMEOUT)
                .build()
                .expect("Clerk HTTP client config must be valid"),
            secret_key,
        }))
    }

    pub(crate) async fn delete_user(&self, clerk_user_id: &str) -> ClerkUserDeletion {
        match self {
            Self::Api(api) => api.delete_user(clerk_user_id).await,
            Self::Unconfigured => {
                ClerkUserDeletion::Retry(format!("{CLERK_SECRET_KEY_ENV} is not set"))
            }
            #[cfg(test)]
            Self::Scripted(clerk) => clerk.delete_user(clerk_user_id),
        }
    }
}

impl ClerkBackendApi {
    async fn delete_user(&self, clerk_user_id: &str) -> ClerkUserDeletion {
        let mut url = reqwest::Url::parse(CLERK_USERS_URL).expect("Clerk users URL must be valid");
        url.path_segments_mut()
            .expect("Clerk users URL has a path")
            .push(clerk_user_id);
        match self
            .client
            .delete(url)
            .bearer_auth(&self.secret_key)
            .send()
            .await
        {
            Ok(response) => classify_clerk_response(response.status().as_u16()),
            Err(error) => {
                ClerkUserDeletion::Retry(format!("Clerk was unreachable: {}", error.without_url()))
            }
        }
    }
}

fn classify_clerk_response(status: u16) -> ClerkUserDeletion {
    match status {
        200..=299 | 404 => ClerkUserDeletion::Deleted,
        _ => ClerkUserDeletion::Retry(format!("Clerk answered {status}")),
    }
}

/// Answers with scripted outcomes, oldest first, then deletes.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct ScriptedClerkUsers {
    pub(crate) scripted: std::sync::Mutex<std::collections::VecDeque<ClerkUserDeletion>>,
    pub(crate) attempts: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl ScriptedClerkUsers {
    fn delete_user(&self, clerk_user_id: &str) -> ClerkUserDeletion {
        self.attempts
            .lock()
            .unwrap()
            .push(clerk_user_id.to_string());
        self.scripted
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(ClerkUserDeletion::Deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_user_counts_as_deleted_and_anything_else_is_retried() {
        for status in [200, 404] {
            assert_eq!(classify_clerk_response(status), ClerkUserDeletion::Deleted);
        }
        for status in [401, 422, 429, 503] {
            assert!(matches!(
                classify_clerk_response(status),
                ClerkUserDeletion::Retry(_)
            ));
        }
    }
}
