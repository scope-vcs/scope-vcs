//! GitHub webhook deliveries. The signature is checked over the raw body
//! before anything is parsed. Every event Scope reacts to is named in
//! `GitHubWebhookEvent::parse`; any other event is acknowledged and dropped.
//! Deliveries can be late or redelivered, so what one reports is confirmed
//! with GitHub before it changes a link.
//! Check deliveries carry no results Scope trusts: they only say which
//! commit to read from GitHub's API again.

use super::{GitHubApp, InstallationStatus};
use crate::error::ApiError;
use hmac::{Hmac, KeyInit, Mac};
use scope_domain::github_connection::GitHubInstallationChange;
use serde::{Deserialize, de::DeserializeOwned};
use sha2::Sha256;
use std::collections::BTreeSet;

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
    /// Something about a commit's checks changed. The delivery only prompts
    /// Scope to read the commit's check runs again, and for a workflow run
    /// delivery, that run.
    ChecksChanged {
        github_repository_id: u64,
        commit_oid: String,
        workflow_run_id: Option<u64>,
    },
    /// A repository was made public or private. Deliveries can arrive out of
    /// order, so Scope asks GitHub which it is now.
    RepositoryVisibilityChanged {
        github_repository_id: u64,
    },
    Ignored,
}

impl GitHubWebhookEvent {
    pub(crate) fn parse(event: &str, body: &[u8]) -> Result<Self, ApiError> {
        match event {
            "repository" => {
                let payload: RepositoryPayload = payload(body)?;
                match (payload.action.as_str(), payload.repository) {
                    ("publicized" | "privatized", Some(repository)) => {
                        Ok(Self::RepositoryVisibilityChanged {
                            github_repository_id: repository.id,
                        })
                    }
                    _ => Ok(Self::Ignored),
                }
            }
            "check_run" | "check_suite" | "workflow_run" => {
                let payload: ChecksPayload = payload(body)?;
                let subject = match event {
                    "check_run" => payload.check_run,
                    "check_suite" => payload.check_suite,
                    _ => payload.workflow_run,
                };
                let (Some(repository), Some(subject)) = (payload.repository, subject) else {
                    return Ok(Self::Ignored);
                };
                if !is_commit_oid(&subject.head_sha) {
                    return Ok(Self::Ignored);
                }
                Ok(Self::ChecksChanged {
                    github_repository_id: repository.id,
                    commit_oid: subject.head_sha,
                    workflow_run_id: subject.id.filter(|_| event == "workflow_run"),
                })
            }
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

/// The part of a reported change GitHub still confirms. An installation that
/// is gone or suspended now is reported as such whatever the delivery said;
/// an active one confirms only repositories it no longer reaches.
pub(crate) async fn confirmed_installation_change(
    app: &GitHubApp,
    installation_id: u64,
    reported: &GitHubInstallationChange,
) -> Result<Option<GitHubInstallationChange>, ApiError> {
    match app.installation_status(installation_id).await? {
        InstallationStatus::Uninstalled => Ok(Some(GitHubInstallationChange::Uninstalled)),
        InstallationStatus::Suspended => Ok(Some(GitHubInstallationChange::Suspended)),
        InstallationStatus::Active => {
            let GitHubInstallationChange::RepositoriesRemoved(removed) = reported else {
                return Ok(None);
            };
            let reachable = app
                .installation_repositories(installation_id)
                .await?
                .into_iter()
                .map(|repository| repository.id)
                .collect::<BTreeSet<_>>();
            let gone = removed
                .difference(&reachable)
                .copied()
                .collect::<BTreeSet<_>>();
            Ok((!gone.is_empty()).then_some(GitHubInstallationChange::RepositoriesRemoved(gone)))
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
struct RepositoryPayload {
    action: String,
    repository: Option<Repository>,
}

/// `check_run`, `check_suite` and `workflow_run` deliveries each name their
/// subject under their own key.
#[derive(Deserialize)]
struct ChecksPayload {
    repository: Option<Repository>,
    check_run: Option<ChecksSubject>,
    check_suite: Option<ChecksSubject>,
    workflow_run: Option<ChecksSubject>,
}

#[derive(Deserialize)]
struct ChecksSubject {
    id: Option<u64>,
    head_sha: String,
}

pub(super) fn is_commit_oid(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

    #[test]
    fn visibility_events_name_the_repository_to_ask_about() {
        let parse = |body: serde_json::Value| {
            GitHubWebhookEvent::parse("repository", body.to_string().as_bytes()).unwrap()
        };
        for action in ["publicized", "privatized"] {
            assert_eq!(
                parse(serde_json::json!({ "action": action, "repository": { "id": 42 } })),
                GitHubWebhookEvent::RepositoryVisibilityChanged {
                    github_repository_id: 42
                }
            );
        }
        assert_eq!(
            parse(serde_json::json!({ "action": "renamed", "repository": { "id": 42 } })),
            GitHubWebhookEvent::Ignored
        );
    }

    #[test]
    fn check_events_name_the_repository_and_commit_to_read_again() {
        let parse = |event: &str, body: serde_json::Value| {
            GitHubWebhookEvent::parse(event, body.to_string().as_bytes()).unwrap()
        };
        let sha = "a".repeat(40);
        for (event, workflow_run_id) in [
            ("check_run", None),
            ("check_suite", None),
            ("workflow_run", Some(9)),
        ] {
            assert_eq!(
                parse(
                    event,
                    serde_json::json!({
                        "action": "completed",
                        "repository": { "id": 42 },
                        (event): { "id": 9, "head_sha": sha },
                    })
                ),
                GitHubWebhookEvent::ChecksChanged {
                    github_repository_id: 42,
                    commit_oid: sha.clone(),
                    workflow_run_id,
                },
                "{event}"
            );
        }
        assert_eq!(
            parse(
                "check_run",
                serde_json::json!({ "repository": { "id": 42 }, "check_run": { "head_sha": "main" } })
            ),
            GitHubWebhookEvent::Ignored
        );
        assert_eq!(
            parse(
                "workflow_run",
                serde_json::json!({ "repository": { "id": 42 }, "check_run": { "head_sha": sha } })
            ),
            GitHubWebhookEvent::Ignored
        );
    }
}
