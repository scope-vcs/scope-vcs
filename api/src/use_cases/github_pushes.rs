//! Sends request branches to GitHub: pushes the commit a request's checks
//! test, and deletes the branch once the request merges or closes. Pushes are
//! claimed with a lease, so one a dying process left half done is taken up by
//! another. Right before git runs, a push checks that its claim still holds
//! and that no newer push of its branch was queued, so an old commit never
//! lands after a newer one. A push that cannot work until someone changes
//! something, such as a disconnected repository, gives up at once instead of
//! retrying.

use crate::{
    auth::tokens::random_token, error::ApiError,
    git::request_refs::with_request_revision_store_repo, persistence::unix_now,
    repo_events::RepoChangeReason, state::AppState,
};
use scope_domain::requests::{GitHubPush, github_push_retry_at};
use scope_postgres::db::{GitHubPushOutcome, GitHubPushStanding};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const BATCH_SIZE: usize = 4;
/// Comfortably longer than reading a revision and pushing it may take, so a
/// live pusher is never raced.
const LEASE_SECS: u64 = 45 * 60;

enum PushFailure {
    Retry(String),
    GiveUp(String),
    /// The claim lapsed or a newer push replaced this one: nothing is sent
    /// and nothing is recorded.
    Stale,
}

impl From<ApiError> for PushFailure {
    fn from(error: ApiError) -> Self {
        Self::Retry(error.into_public_message())
    }
}

/// Runs the pushes this process can claim, one at a time. Each is leased
/// right before it runs, so no lease runs out while earlier pushes take their
/// time. Returns how many it claimed.
pub(crate) async fn push_due_github_branches(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let requests = state.metadata.requests();
    let mut claimed = 0;
    while claimed < BATCH_SIZE {
        let claim_token =
            random_token("github_push_claim_", "failed to generate push claim token")?;
        let Some(push) = requests
            .claim_due_github_pushes(
                &claim_token,
                now_unix,
                now_unix.saturating_add(LEASE_SECS),
                1,
            )
            .await?
            .pop()
        else {
            break;
        };
        claimed += 1;
        run_claimed_push(state, &push, &claim_token, now_unix).await;
    }
    Ok(claimed)
}

/// Sends one claimed push and records how it ended while the claim holds.
pub(crate) async fn run_claimed_push(
    state: &AppState,
    push: &GitHubPush,
    claim_token: &str,
    now_unix: u64,
) {
    let outcome = match send(state, push, claim_token).await {
        Ok(()) => GitHubPushOutcome::Succeeded,
        Err(PushFailure::Retry(error)) => GitHubPushOutcome::Failed {
            error,
            retry_at_unix: github_push_retry_at(push.attempts, now_unix),
        },
        Err(PushFailure::GiveUp(error)) => GitHubPushOutcome::Failed {
            error,
            retry_at_unix: None,
        },
        Err(PushFailure::Stale) => return,
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
    match state
        .metadata
        .requests()
        .finish_github_push(&push.id, claim_token, outcome, now_unix)
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

/// Asked right before git runs. A push whose claim lapsed sends nothing, and
/// one a newer push of its branch replaces is dropped unsent.
async fn ensure_current(
    state: &AppState,
    push_id: &str,
    claim_token: &str,
) -> Result<(), PushFailure> {
    let requests = state.metadata.requests();
    match requests
        .github_push_standing(push_id, claim_token)
        .await
        .map_err(ApiError::from)?
    {
        GitHubPushStanding::Current => Ok(()),
        GitHubPushStanding::Superseded => {
            requests
                .drop_superseded_github_push(push_id, claim_token)
                .await
                .map_err(ApiError::from)?;
            Err(PushFailure::Stale)
        }
        GitHubPushStanding::Lost => Err(PushFailure::Stale),
    }
}

async fn send(state: &AppState, push: &GitHubPush, claim_token: &str) -> Result<(), PushFailure> {
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
        ensure_current(state, &push.id, claim_token).await?;
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
    // way merge preparation reads a request head. Filling it may take a
    // while, so the claim is checked once the commit is at hand.
    let (check_state, push_id, claim_token) =
        (state.clone(), push.id.clone(), claim_token.to_string());
    with_request_revision_store_repo(state, &incarnation, &request, &revision, move |repo, _| {
        Ok(
            crate::git::blocking::block_on(ensure_current(&check_state, &push_id, &claim_token))
                .and_then(|()| {
                    remote
                        .push(repo, &target_oid, &git_ref)
                        .map_err(PushFailure::Retry)
                }),
        )
    })
    .await?
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
