//! Sends request branches to GitHub: pushes the commit a request's checks
//! test, and deletes the branch once the request merges or closes. Pushes are
//! claimed with a lease, so one a dying process left half done is taken up by
//! another. A push that cannot work until someone changes something, such as
//! a disconnected repository, gives up at once instead of retrying.

use crate::{
    auth::tokens::random_token, error::ApiError,
    git::request_refs::with_request_revision_store_repo, persistence::unix_now,
    repo_events::RepoChangeReason, state::AppState,
};
use scope_domain::requests::{GitHubPush, github_push_retry_at};
use scope_postgres::db::GitHubPushOutcome;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const BATCH_SIZE: u64 = 4;
/// Longer than a push may take, so a live pusher is never raced.
const LEASE_SECS: u64 = 15 * 60;

enum PushFailure {
    Retry(String),
    GiveUp(String),
}

impl From<ApiError> for PushFailure {
    fn from(error: ApiError) -> Self {
        Self::Retry(error.into_public_message())
    }
}

/// Runs every push this process can claim. Returns how many it claimed.
pub(crate) async fn push_due_github_branches(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let claim_token = random_token("github_push_claim_", "failed to generate push claim token")?;
    let requests = state.metadata.requests();
    let claimed = requests
        .claim_due_github_pushes(
            &claim_token,
            now_unix,
            now_unix.saturating_add(LEASE_SECS),
            BATCH_SIZE,
        )
        .await?;
    for push in &claimed {
        let outcome = match send(state, push).await {
            Ok(()) => GitHubPushOutcome::Succeeded,
            Err(PushFailure::Retry(error)) => GitHubPushOutcome::Failed {
                error,
                retry_at_unix: github_push_retry_at(push.attempts, now_unix),
            },
            Err(PushFailure::GiveUp(error)) => GitHubPushOutcome::Failed {
                error,
                retry_at_unix: None,
            },
        };
        if let GitHubPushOutcome::Failed { error, .. } = &outcome {
            tracing::warn!(
                push_id = push.id,
                request_id = push.request_id,
                attempts = push.attempts,
                %error,
                "sending a request branch to GitHub failed"
            );
        }
        match requests
            .finish_github_push(&push.id, &claim_token, outcome, now_unix)
            .await
        {
            Ok(Some(_)) => publish_push_change(state, &push.repo_id).await,
            // Another process took the push over after this claim lapsed.
            Ok(None) => {}
            Err(error) => tracing::warn!(
                push_id = push.id,
                error = %error,
                "could not record a GitHub push; its claim will lapse"
            ),
        }
    }
    Ok(claimed.len())
}

async fn send(state: &AppState, push: &GitHubPush) -> Result<(), PushFailure> {
    let app = state.github.clone().ok_or_else(|| {
        PushFailure::GiveUp("GitHub is not configured on this server.".to_string())
    })?;
    let connection = state
        .metadata
        .repositories()
        .github_connection(&push.repo_id)
        .await
        .map_err(ApiError::from)?
        .map(|read| read.connection)
        .filter(|connection| connection.is_connected())
        .ok_or_else(|| {
            PushFailure::GiveUp(
                "This repository is no longer connected to GitHub. Reconnect it in repository settings."
                    .to_string(),
            )
        })?;
    let token = app
        .installation_token(connection.installation_id)
        .await?
        .ok_or_else(|| {
            PushFailure::GiveUp(
                "The Scope GitHub App is no longer installed for this repository.".to_string(),
            )
        })?;
    let remote = app.push_remote(&connection.github_full_name, &token);
    let git_ref = push.git_ref();
    let Some(target_oid) = push.target_oid.clone() else {
        return crate::git::blocking::run(move || Ok(remote.delete(&git_ref)))
            .await?
            .map_err(PushFailure::Retry);
    };
    let requests = state.metadata.requests();
    let gone = || PushFailure::GiveUp("Scope no longer has this revision.".to_string());
    let request = requests
        .request_by_id(&push.request_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(gone)?;
    let revision = requests
        .request_revision_with_head(&request.id, &target_oid)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(gone)?;
    let incarnation = state
        .metadata
        .repositories()
        .repository_record(&push.repo_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(gone)?
        .incarnation();
    // The revision store holds the commit with the history it builds on, the
    // way merge preparation reads a request head.
    with_request_revision_store_repo(state, &incarnation, &request, &revision, move |repo, _| {
        Ok(remote.push(repo, &target_oid, &git_ref))
    })
    .await?
    .map_err(PushFailure::Retry)
}

async fn publish_push_change(state: &AppState, repo_id: &str) {
    match state
        .metadata
        .repositories()
        .repository_record(repo_id)
        .await
    {
        Ok(Some(record)) => {
            state
                .publish_request_summary_refresh(
                    &record.incarnation(),
                    RepoChangeReason::RequestChecksUpdated,
                )
                .await;
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(
            repo_id,
            error = %error,
            "could not announce a GitHub push"
        ),
    }
}

impl AppState {
    pub(crate) fn start_github_pushes(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                let pass = async {
                    state.metadata.admin().readiness_check().await?;
                    push_due_github_branches(&state, unix_now()?).await
                };
                if let Err(error) = pass.await {
                    tracing::warn!(
                        error = %error.operator_diagnostic(),
                        "GitHub push pass failed; retrying"
                    );
                }
                tokio::select! {
                    _ = state.github_push_wakeup.notified() => {},
                    _ = tokio::time::sleep(POLL_INTERVAL) => {},
                }
            }
        });
    }
}
