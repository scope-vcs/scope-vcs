//! GitHub webhook deliveries. The signature is checked over the raw body
//! before anything is parsed. Every event Scope reacts to is named in
//! `GitHubWebhookEvent::parse`; any other event is acknowledged and dropped.

use crate::error::ApiError;
use hmac::{Hmac, KeyInit, Mac};
use scope_domain::github_connection::GitHubInstallationChange;
use serde::{Deserialize, de::DeserializeOwned};
use sha2::Sha256;

pub(crate) const GITHUB_SIGNATURE_HEADER: &str = "x-hub-signature-256";
pub(crate) const GITHUB_EVENT_HEADER: &str = "x-github-event";

/// Whether `X-Hub-Signature-256` is the HMAC-SHA256 of the body under the
/// webhook secret. The comparison is constant time.
pub(crate) fn signature_matches(secret: &[u8], signature: Option<&str>, body: &[u8]) -> bool {
    let Some(signature) = signature
        .and_then(|value| value.strip_prefix("sha256="))
        .and_then(|value| hex::decode(value).ok())
    else {
        return false;
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts keys of any length");
    mac.update(body);
    mac.verify_slice(&signature).is_ok()
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GitHubWebhookEvent {
    InstallationChanged {
        installation_id: u64,
        change: GitHubInstallationChange,
    },
    Ignored,
}

impl GitHubWebhookEvent {
    pub(crate) fn parse(event: &str, body: &[u8]) -> Result<Self, ApiError> {
        match event {
            "installation" => {
                let payload: InstallationPayload = payload(body)?;
                let change = match payload.action.as_str() {
                    "deleted" => GitHubInstallationChange::Uninstalled,
                    "suspend" => GitHubInstallationChange::Suspended,
                    // A link disconnected by a suspension is reconnected by a
                    // maintainer, who can confirm the repository is still there.
                    _ => return Ok(Self::Ignored),
                };
                Ok(Self::InstallationChanged {
                    installation_id: payload.installation.id,
                    change,
                })
            }
            "installation_repositories" => {
                let payload: InstallationRepositoriesPayload = payload(body)?;
                if payload.action != "removed" {
                    return Ok(Self::Ignored);
                }
                Ok(Self::InstallationChanged {
                    installation_id: payload.installation.id,
                    change: GitHubInstallationChange::RepositoriesRemoved(
                        payload
                            .repositories_removed
                            .into_iter()
                            .map(|repository| repository.id)
                            .collect(),
                    ),
                })
            }
            _ => Ok(Self::Ignored),
        }
    }
}

#[derive(Deserialize)]
struct InstallationPayload {
    action: String,
    installation: Installation,
}

#[derive(Deserialize)]
struct InstallationRepositoriesPayload {
    action: String,
    installation: Installation,
    #[serde(default)]
    repositories_removed: Vec<Repository>,
}

#[derive(Deserialize)]
struct Installation {
    id: u64,
}

#[derive(Deserialize)]
struct Repository {
    id: u64,
}

fn payload<T: DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|error| {
        ApiError::bad_request(format!("malformed GitHub webhook payload: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn sign(secret: &[u8], body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn only_the_secret_holders_signature_over_this_body_is_accepted() {
        let body = br#"{"action":"deleted"}"#;
        let signature = sign(b"secret", body);
        assert!(signature_matches(b"secret", Some(&signature), body));
        assert!(!signature_matches(b"other", Some(&signature), body));
        assert!(!signature_matches(b"secret", Some(&signature), b"{}"));
        assert!(!signature_matches(b"secret", None, body));
        assert!(!signature_matches(
            b"secret",
            Some(signature.trim_start_matches("sha256=")),
            body
        ));
        assert!(!signature_matches(b"secret", Some("sha256=zz"), body));
    }

    #[test]
    fn installation_events_become_domain_changes() {
        let parse = |event: &str, body: &str| GitHubWebhookEvent::parse(event, body.as_bytes());
        assert_eq!(
            parse(
                "installation",
                r#"{"action":"deleted","installation":{"id":7}}"#
            )
            .unwrap(),
            GitHubWebhookEvent::InstallationChanged {
                installation_id: 7,
                change: GitHubInstallationChange::Uninstalled,
            }
        );
        assert_eq!(
            parse(
                "installation",
                r#"{"action":"suspend","installation":{"id":7}}"#
            )
            .unwrap(),
            GitHubWebhookEvent::InstallationChanged {
                installation_id: 7,
                change: GitHubInstallationChange::Suspended,
            }
        );
        assert_eq!(
            parse(
                "installation_repositories",
                r#"{"action":"removed","installation":{"id":7},
                    "repositories_removed":[{"id":42,"full_name":"octo/repo"}]}"#
            )
            .unwrap(),
            GitHubWebhookEvent::InstallationChanged {
                installation_id: 7,
                change: GitHubInstallationChange::RepositoriesRemoved(BTreeSet::from([42])),
            }
        );
        for (event, body) in [
            (
                "installation",
                r#"{"action":"unsuspend","installation":{"id":7}}"#,
            ),
            (
                "installation",
                r#"{"action":"created","installation":{"id":7}}"#,
            ),
            (
                "installation_repositories",
                r#"{"action":"added","installation":{"id":7}}"#,
            ),
            ("check_run", "{}"),
            ("ping", "not json"),
        ] {
            assert_eq!(parse(event, body).unwrap(), GitHubWebhookEvent::Ignored);
        }
        assert!(parse("installation", "{}").is_err());
    }
}
