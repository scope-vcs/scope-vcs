use super::{GitHubApp, InstallationStatus};
use crate::error::ApiError;
use hmac::{Hmac, KeyInit, Mac};
use scope_domain::github_connection::GitHubInstallationChange;
use serde::Deserialize;
use sha2::Sha256;
use std::collections::BTreeSet;

mod reviews;
pub(crate) use reviews::GitHubReviewDelivery;

pub(crate) const GITHUB_SIGNATURE_HEADER: &str = "x-hub-signature-256";
pub(crate) const GITHUB_EVENT_HEADER: &str = "x-github-event";

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
    ChecksChanged {
        github_repository_id: u64,
        commit_oid: String,
        workflow_run_id: Option<u64>,
    },
    WorkflowJobChanged {
        github_repository_id: u64,
        job_id: u64,
    },
    RepositoryVisibilityChanged {
        github_repository_id: u64,
    },
    ReviewActivity(GitHubReviewDelivery),
    Ignored,
}

impl GitHubWebhookEvent {
    pub(crate) fn parse(event: &str, body: &[u8]) -> Result<Self, ApiError> {
        if reviews::EVENTS.contains(&event) {
            return Ok(reviews::parse(event, body)?.map_or(Self::Ignored, Self::ReviewActivity));
        }
        if !matches!(
            event,
            "repository"
                | "check_run"
                | "check_suite"
                | "workflow_run"
                | "workflow_job"
                | "installation"
                | "installation_repositories"
        ) {
            return Ok(Self::Ignored);
        }
        let payload: Payload = serde_json::from_slice(body).map_err(malformed)?;
        let installation_id = || -> Result<u64, ApiError> {
            Ok(required(payload.installation.as_ref(), "installation")?.id)
        };
        Ok(match event {
            "repository" => match (
                required(payload.action.as_deref(), "action")?,
                payload.repository,
            ) {
                ("publicized" | "privatized", Some(repository)) => {
                    Self::RepositoryVisibilityChanged {
                        github_repository_id: repository.id,
                    }
                }
                _ => Self::Ignored,
            },
            "check_run" | "check_suite" | "workflow_run" => {
                let subject = match event {
                    "check_run" => payload.check_run,
                    "check_suite" => payload.check_suite,
                    _ => payload.workflow_run,
                };
                match (payload.repository, subject) {
                    (Some(repository), Some(subject)) if is_commit_oid(&subject.head_sha) => {
                        Self::ChecksChanged {
                            github_repository_id: repository.id,
                            commit_oid: subject.head_sha,
                            workflow_run_id: subject.id.filter(|_| event == "workflow_run"),
                        }
                    }
                    _ => Self::Ignored,
                }
            }
            "workflow_job" => match (payload.repository, payload.workflow_job) {
                (Some(repository), Some(job)) => Self::WorkflowJobChanged {
                    github_repository_id: repository.id,
                    job_id: job.id,
                },
                _ => Self::Ignored,
            },
            "installation" => {
                let change = match required(payload.action.as_deref(), "action")? {
                    "deleted" => GitHubInstallationChange::Uninstalled,
                    "suspend" => GitHubInstallationChange::Suspended,
                    _ => return Ok(Self::Ignored),
                };
                Self::InstallationChanged {
                    installation_id: installation_id()?,
                    change,
                }
            }
            _ => {
                if required(payload.action.as_deref(), "action")? != "removed" {
                    return Ok(Self::Ignored);
                }
                Self::InstallationChanged {
                    installation_id: installation_id()?,
                    change: GitHubInstallationChange::RepositoriesRemoved(
                        payload
                            .repositories_removed
                            .iter()
                            .map(|repository| repository.id)
                            .collect(),
                    ),
                }
            }
        })
    }
}

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
struct Payload {
    action: Option<String>,
    installation: Option<Id>,
    repository: Option<Id>,
    #[serde(default)]
    repositories_removed: Vec<Id>,
    check_run: Option<ChecksSubject>,
    check_suite: Option<ChecksSubject>,
    workflow_run: Option<ChecksSubject>,
    workflow_job: Option<Id>,
}

#[derive(Deserialize)]
struct Id {
    id: u64,
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

fn required<T>(value: Option<T>, field: &str) -> Result<T, ApiError> {
    value.ok_or_else(|| malformed(format!("missing field `{field}`")))
}

fn malformed(error: impl std::fmt::Display) -> ApiError {
    ApiError::bad_request(format!("malformed GitHub webhook payload: {error}"))
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
    fn job_events_name_the_repository_and_job_to_read_again() {
        let parse = |body: serde_json::Value| {
            GitHubWebhookEvent::parse("workflow_job", body.to_string().as_bytes()).unwrap()
        };
        assert_eq!(
            parse(serde_json::json!({
                "action": "in_progress",
                "repository": { "id": 42 },
                "workflow_job": { "id": 7, "run_id": 9 },
            })),
            GitHubWebhookEvent::WorkflowJobChanged {
                github_repository_id: 42,
                job_id: 7
            }
        );
        assert_eq!(
            parse(serde_json::json!({ "action": "queued", "repository": { "id": 42 } })),
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
