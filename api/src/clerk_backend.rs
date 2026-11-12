use crate::{
    auth::clerk::AudienceClaim,
    config::{CLERK_SECRET_KEY_ENV, non_empty_env},
};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};

pub(crate) const CLERK_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

const CLERK_USERS_URL: &str = "https://api.clerk.com/v1/users";
const CLERK_OAUTH_TOKEN_VERIFY_URL: &str =
    "https://api.clerk.com/v1/oauth_applications/access_tokens/verify";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClerkOAuthAccessToken {
    pub(crate) subject: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) audiences: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClerkOAuthTokenCheck {
    Active(ClerkOAuthAccessToken),
    Inactive,
    Unavailable(String),
}

#[derive(Deserialize)]
struct VerifiedOAuthAccessToken {
    subject: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    aud: Option<AudienceClaim>,
    revoked: bool,
    expired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClerkUserDeletion {
    Deleted,
    Retry(String),
}

#[derive(Clone)]
pub(crate) enum ClerkBackend {
    Api(Arc<ClerkBackendApi>),
    Unconfigured,
    #[cfg(test)]
    Scripted(Arc<ScriptedClerkBackend>),
}

pub(crate) struct ClerkBackendApi {
    client: reqwest::Client,
    secret_key: String,
}

impl ClerkBackend {
    pub(crate) fn from_env() -> Self {
        let Some(secret_key) = non_empty_env(CLERK_SECRET_KEY_ENV) else {
            tracing::warn!(
                "{CLERK_SECRET_KEY_ENV} is not set; deleted accounts keep their Clerk users and agent tokens cannot be verified until it is"
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

    pub(crate) async fn verify_oauth_access_token(&self, token: &str) -> ClerkOAuthTokenCheck {
        match self {
            Self::Api(api) => api.verify_oauth_access_token(token).await,
            Self::Unconfigured => {
                ClerkOAuthTokenCheck::Unavailable(format!("{CLERK_SECRET_KEY_ENV} is not set"))
            }
            #[cfg(test)]
            Self::Scripted(clerk) => clerk.verify_oauth_access_token(token),
        }
    }
}

impl ClerkBackendApi {
    async fn delete_user(&self, clerk_user_id: &str) -> ClerkUserDeletion {
        let mut url = reqwest::Url::parse(CLERK_USERS_URL).expect("Clerk users URL must be valid");
        url.path_segments_mut()
            .expect("Clerk users URL has a path")
            .push(clerk_user_id);
        match scope_service_runtime::outbound_http::send_traced(
            self.client.delete(url).bearer_auth(&self.secret_key),
            "/v1/users/{user_id}",
        )
        .await
        {
            Ok(response) => classify_clerk_response(response.status().as_u16()),
            Err(error) => {
                ClerkUserDeletion::Retry(format!("Clerk was unreachable: {}", error.without_url()))
            }
        }
    }
}

impl ClerkBackendApi {
    async fn verify_oauth_access_token(&self, token: &str) -> ClerkOAuthTokenCheck {
        let response = match scope_service_runtime::outbound_http::send_traced(
            self.client
                .post(CLERK_OAUTH_TOKEN_VERIFY_URL)
                .bearer_auth(&self.secret_key)
                .json(&serde_json::json!({ "access_token": token })),
            "/v1/oauth_applications/access_tokens/verify",
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                return ClerkOAuthTokenCheck::Unavailable(format!(
                    "Clerk was unreachable: {}",
                    error.without_url()
                ));
            }
        };
        let status = response.status().as_u16();
        match status {
            200 => match response.json::<VerifiedOAuthAccessToken>().await {
                Ok(token) => token.into_check(),
                Err(error) => ClerkOAuthTokenCheck::Unavailable(format!(
                    "Clerk answered an unreadable token: {error}"
                )),
            },
            400 | 404 | 422 => ClerkOAuthTokenCheck::Inactive,
            _ => ClerkOAuthTokenCheck::Unavailable(format!("Clerk answered {status}")),
        }
    }
}

impl VerifiedOAuthAccessToken {
    fn into_check(self) -> ClerkOAuthTokenCheck {
        if self.revoked || self.expired {
            return ClerkOAuthTokenCheck::Inactive;
        }
        ClerkOAuthTokenCheck::Active(ClerkOAuthAccessToken {
            subject: self.subject,
            scopes: self.scopes,
            audiences: match self.aud {
                Some(AudienceClaim::One(audience)) => vec![audience],
                Some(AudienceClaim::Many(audiences)) => audiences,
                None => Vec::new(),
            },
        })
    }
}

fn classify_clerk_response(status: u16) -> ClerkUserDeletion {
    match status {
        200..=299 | 404 => ClerkUserDeletion::Deleted,
        _ => ClerkUserDeletion::Retry(format!("Clerk answered {status}")),
    }
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct ScriptedClerkBackend {
    pub(crate) scripted: std::sync::Mutex<std::collections::VecDeque<ClerkUserDeletion>>,
    pub(crate) attempts: std::sync::Mutex<Vec<String>>,
    pub(crate) oauth_tokens:
        std::sync::Mutex<std::collections::HashMap<String, ClerkOAuthTokenCheck>>,
}

#[cfg(test)]
impl ScriptedClerkBackend {
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

    fn verify_oauth_access_token(&self, token: &str) -> ClerkOAuthTokenCheck {
        self.oauth_tokens
            .lock()
            .unwrap()
            .get(token)
            .cloned()
            .unwrap_or(ClerkOAuthTokenCheck::Inactive)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verified_token_reads_its_audience_in_any_shape_clerk_sends() {
        for (aud, expected) in [
            (
                serde_json::json!("https://api.example/mcp"),
                vec!["https://api.example/mcp"],
            ),
            (
                serde_json::json!(["https://api.example/mcp"]),
                vec!["https://api.example/mcp"],
            ),
            (serde_json::Value::Null, vec![]),
        ] {
            let verified: VerifiedOAuthAccessToken = serde_json::from_value(serde_json::json!({
                "subject": "user_agent",
                "scopes": ["read"],
                "aud": aud,
                "revoked": false,
                "expired": false,
            }))
            .unwrap();
            let ClerkOAuthTokenCheck::Active(token) = verified.into_check() else {
                panic!("an unrevoked, unexpired token is active");
            };
            assert_eq!(token.audiences, expected);
        }
    }

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
