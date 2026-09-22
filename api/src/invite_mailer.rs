//! Sends repository invite emails through Resend.

use crate::config::non_empty_env;
use scope_domain::repo_invite_email::InviteEmailAttempt;
use std::{sync::Arc, time::Duration};

pub(crate) const SCOPE_RESEND_API_KEY_ENV: &str = "SCOPE_RESEND_API_KEY";
pub(crate) const SCOPE_INVITE_EMAIL_FROM_ENV: &str = "SCOPE_INVITE_EMAIL_FROM";
const DEFAULT_FROM: &str = "Scope <invites@scopevcs.com>";
const RESEND_EMAILS_URL: &str = "https://api.resend.com/emails";

pub(crate) struct InviteEmailMessage {
    /// Stable per email, so a repeated attempt cannot send a second copy.
    pub(crate) idempotency_key: String,
    pub(crate) to: String,
    pub(crate) reply_to: String,
    pub(crate) subject: String,
    pub(crate) text: String,
    pub(crate) html: String,
}

pub(crate) struct InviteEmailOutcome {
    pub(crate) attempt: InviteEmailAttempt,
    pub(crate) provider_message_id: Option<String>,
}

#[derive(Clone)]
pub(crate) enum InviteMailer {
    Resend(Arc<ResendMailer>),
    /// No API key is configured. Emails fail at once so the owner sees
    /// "Delivery failed" and copies a link, instead of waiting on a queue
    /// nobody drains.
    Unconfigured,
    #[cfg(test)]
    Recording(Arc<RecordingMailer>),
}

pub(crate) struct ResendMailer {
    client: reqwest::Client,
    api_key: String,
    from: String,
}

impl InviteMailer {
    pub(crate) fn from_env() -> Self {
        let Some(api_key) = non_empty_env(SCOPE_RESEND_API_KEY_ENV) else {
            tracing::warn!("{SCOPE_RESEND_API_KEY_ENV} is not set; invite emails will fail");
            return Self::Unconfigured;
        };
        Self::Resend(Arc::new(ResendMailer {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("invite mailer HTTP client config must be valid"),
            api_key,
            from: non_empty_env(SCOPE_INVITE_EMAIL_FROM_ENV)
                .unwrap_or_else(|| DEFAULT_FROM.to_string()),
        }))
    }

    pub(crate) async fn send(&self, message: &InviteEmailMessage) -> InviteEmailOutcome {
        match self {
            Self::Resend(mailer) => mailer.send(message).await,
            Self::Unconfigured => InviteEmailOutcome {
                attempt: InviteEmailAttempt::Refused("email delivery is not configured".into()),
                provider_message_id: None,
            },
            #[cfg(test)]
            Self::Recording(mailer) => mailer.send(message),
        }
    }
}

impl ResendMailer {
    async fn send(&self, message: &InviteEmailMessage) -> InviteEmailOutcome {
        let response = self
            .client
            .post(RESEND_EMAILS_URL)
            .bearer_auth(&self.api_key)
            .header("Idempotency-Key", &message.idempotency_key)
            .json(&serde_json::json!({
                "from": self.from,
                "to": [message.to],
                "reply_to": message.reply_to,
                "subject": message.subject,
                "text": message.text,
                "html": message.html,
            }))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                return outcome(InviteEmailAttempt::Retryable(format!(
                    "Resend was unreachable: {}",
                    error.without_url()
                )));
            }
        };
        let status = response.status();
        let body = response
            .json::<serde_json::Value>()
            .await
            .unwrap_or_default();
        classify_resend_response(status.as_u16(), &body)
    }
}

fn outcome(attempt: InviteEmailAttempt) -> InviteEmailOutcome {
    InviteEmailOutcome {
        attempt,
        provider_message_id: None,
    }
}

fn classify_resend_response(status: u16, body: &serde_json::Value) -> InviteEmailOutcome {
    let error_name = body["name"].as_str().unwrap_or_default();
    let detail = || {
        let message = body["message"].as_str().unwrap_or("no detail");
        format!("Resend answered {status} {error_name}: {message}")
    };
    match status {
        200..=299 => InviteEmailOutcome {
            attempt: InviteEmailAttempt::Accepted,
            provider_message_id: body["id"].as_str().map(str::to_string),
        },
        // An earlier attempt with this key already handed Resend the email.
        // This attempt carried a newer link, so its payload differs.
        409 if error_name == "invalid_idempotent_request" => outcome(InviteEmailAttempt::Accepted),
        408 | 409 | 425 | 429 | 500..=599 => outcome(InviteEmailAttempt::Retryable(detail())),
        _ => outcome(InviteEmailAttempt::Refused(detail())),
    }
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct RecordingMailer {
    pub(crate) sent: std::sync::Mutex<Vec<(String, String, String)>>,
    /// Outcomes to hand back before accepting, oldest first.
    pub(crate) scripted: std::sync::Mutex<std::collections::VecDeque<InviteEmailAttempt>>,
}

#[cfg(test)]
impl RecordingMailer {
    fn send(&self, message: &InviteEmailMessage) -> InviteEmailOutcome {
        let attempt = self
            .scripted
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(InviteEmailAttempt::Accepted);
        if attempt == InviteEmailAttempt::Accepted {
            self.sent.lock().unwrap().push((
                message.to.clone(),
                message.reply_to.clone(),
                message.text.clone(),
            ));
        }
        InviteEmailOutcome {
            provider_message_id: (attempt == InviteEmailAttempt::Accepted)
                .then(|| format!("test_{}", message.idempotency_key)),
            attempt,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resend_answers_map_to_accept_retry_or_refuse() {
        let accepted = classify_resend_response(200, &json!({ "id": "re_1" }));
        assert_eq!(accepted.attempt, InviteEmailAttempt::Accepted);
        assert_eq!(accepted.provider_message_id.as_deref(), Some("re_1"));

        let replayed =
            classify_resend_response(409, &json!({ "name": "invalid_idempotent_request" }));
        assert_eq!(replayed.attempt, InviteEmailAttempt::Accepted);

        for (status, name) in [
            (409, "concurrent_idempotent_requests"),
            (429, "rate_limit_exceeded"),
            (503, ""),
        ] {
            let answer = classify_resend_response(status, &json!({ "name": name }));
            assert!(
                matches!(answer.attempt, InviteEmailAttempt::Retryable(_)),
                "{status}"
            );
        }
        for status in [401, 403, 422] {
            let answer = classify_resend_response(status, &json!({ "message": "nope" }));
            assert!(
                matches!(answer.attempt, InviteEmailAttempt::Refused(_)),
                "{status}"
            );
        }
    }
}
