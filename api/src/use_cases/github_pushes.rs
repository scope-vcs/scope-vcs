//! Sends Scope's branches to GitHub: pushes the commit a request's checks
//! test, and deletes the branch once the request merges or closes, and pushes
//! main for a connection test and deletes it once the test ends. Pushes are
//! claimed with a lease, so one a dying process left half done is taken up by
//! another. Right before git runs, a push checks that its claim still holds
//! and that no newer push of its branch was queued, so an old commit never
//! lands after a newer one. A push that cannot work until someone changes
//! something, such as a disconnected repository, gives up at once instead of
//! retrying.

use crate::{
    auth::tokens::random_token, error::ApiError,
    git::request_refs::with_request_revision_store_repo, github::GitHubApp, persistence::unix_now,
    repo_events::RepoChangeReason, state::AppState,
    use_cases::public_check_commits::with_check_commit,
};
use scope_domain::{
    github_connection::{GitHubConnection, PRIVATE_REQUESTS_WITHHELD_MESSAGE},
    requests::{GitHubBranch, GitHubPush, RequestAudience, github_push_retry_at},
};
use scope_postgres::db::{GitHubPushOutcome, GitHubPushStanding};
use std::path::Path;
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
    let sent = send(state, push, claim_token).await;
    // A push can take minutes, and when it ended is when GitHub got the
    // branch, which is what waiting for workflows counts from.
    let now_unix = unix_now().map_or(now_unix, |finished| finished.max(now_unix));
    let outcome = match sent {
        Ok(()) => GitHubPushOutcome::Succeeded,
        Err(PushFailure::Retry(error)) => GitHubPushOutcome::Failed {
            error,
            retry_at_unix: github_push_retry_at(push, now_unix),
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
            branch = push.branch.name(),
            attempts = push.attempts,
            %error,
            "sending a branch to GitHub failed"
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
/// one a newer push of its branch replaces is dropped unsent. A commit is
/// pushed only while the repository is still connected the way it was when
/// the push was queued, and a private request's commit only while that
/// repository may receive private requests. A deletion runs wherever the
/// branch was pushed, even once the Scope repository is gone.
async fn ensure_current(
    state: &AppState,
    push: &GitHubPush,
    claim_token: &str,
    audience: Option<RequestAudience>,
) -> Result<(), PushFailure> {
    let requests = state.metadata.requests();
    match requests
        .github_push_standing(&push.id, claim_token)
        .await
        .map_err(ApiError::from)?
    {
        GitHubPushStanding::Current => {}
        GitHubPushStanding::Superseded => {
            requests
                .drop_superseded_github_push(&push.id, claim_token)
                .await
                .map_err(ApiError::from)?;
            return Err(PushFailure::Stale);
        }
        GitHubPushStanding::Lost => return Err(PushFailure::Stale),
    }
    if push.target_oid.is_none() {
        return Ok(());
    }
    let connection = state
        .metadata
        .repositories()
        .github_connection(&push.repo_id)
        .await
        .map_err(ApiError::from)?
        .map(|read| read.connection)
        .filter(|connection| push.destination.is_connected_through(connection))
        .ok_or_else(|| {
            PushFailure::GiveUp(
                "This repository is no longer connected to that GitHub repository. Reconnect it in repository settings."
                    .to_string(),
            )
        })?;
    ensure_may_receive(&connection, audience)
}

fn ensure_may_receive(
    connection: &GitHubConnection,
    audience: Option<RequestAudience>,
) -> Result<(), PushFailure> {
    if audience == Some(RequestAudience::Private) && !connection.may_receive_private_requests() {
        return Err(PushFailure::GiveUp(
            PRIVATE_REQUESTS_WITHHELD_MESSAGE.to_string(),
        ));
    }
    Ok(())
}

/// Asks GitHub whether the connected repository is public now and records
/// what it says. Returns the link as it stands after.
pub(crate) async fn refresh_github_visibility(
    state: &AppState,
    connection: &GitHubConnection,
) -> Result<GitHubConnection, ApiError> {
    let app = state
        .github
        .as_deref()
        .ok_or_else(|| ApiError::not_found("GitHub is not configured on this server"))?;
    let repository = app
        .installation_repository(connection.installation_id, connection.github_repository_id)
        .await?
        .ok_or_else(|| {
            ApiError::upstream_unavailable(
                "The Scope GitHub App can no longer reach this GitHub repository.",
                format!(
                    "installation {} no longer lists {}",
                    connection.installation_id, connection.github_full_name
                ),
            )
        })?;
    let mut refreshed = connection.clone();
    if refreshed.apply_visibility(repository.private)
        && let Some(incarnation) = state
            .metadata
            .repositories()
            .apply_github_repository_visibility(connection.github_repository_id, repository.private)
            .await?
    {
        state
            .publish_request_summary_refresh(
                &incarnation,
                RepoChangeReason::GitHubConnectionChanged,
            )
            .await;
    }
    Ok(refreshed)
}

async fn send(state: &AppState, push: &GitHubPush, claim_token: &str) -> Result<(), PushFailure> {
    let app = state.github.clone().ok_or_else(|| {
        PushFailure::GiveUp("GitHub is not configured on this server.".to_string())
    })?;
    let destination = &push.destination;
    let Some(target_oid) = push.target_oid.clone() else {
        ensure_current(state, push, claim_token, None).await?;
        let token = installation_token(&app, destination.installation_id).await?;
        let remote = app.push_remote(&destination.github_full_name, &token);
        let git_ref = push.branch.git_ref();
        return crate::git::blocking::run(move || Ok(remote.delete(&git_ref)))
            .await?
            .map_err(PushFailure::Retry);
    };
    let requests = state.metadata.requests();
    let gone = || PushFailure::GiveUp("Scope no longer has this revision.".to_string());
    // A request's branch carries the commit its evaluation tests.
    let tested = match &push.branch {
        GitHubBranch::Request(request_id) => {
            let request = requests
                .request_by_id(request_id)
                .await
                .map_err(ApiError::from)?
                .ok_or_else(gone)?;
            let evaluation = requests
                .request_check_evaluation_testing(&request.id, &target_oid)
                .await
                .map_err(ApiError::from)?
                .ok_or_else(gone)?;
            Some((request, evaluation))
        }
        GitHubBranch::SetupCheck => None,
    };
    // Main holds private files, and a public contribution's check commit is
    // built on it, so both go only where a private request may.
    let audience = Some(
        tested
            .as_ref()
            .map_or(RequestAudience::Private, |(request, evaluation)| {
                evaluation.tested_code_audience(request.audience)
            }),
    );
    ensure_current(state, push, claim_token, audience).await?;
    // A webhook may never have said the repository became public, so GitHub
    // is asked before private content is sent there.
    if audience == Some(RequestAudience::Private)
        && let Some(connection) = state
            .metadata
            .repositories()
            .github_connection(&push.repo_id)
            .await
            .map_err(ApiError::from)?
    {
        ensure_may_receive(
            &refresh_github_visibility(state, &connection.connection).await?,
            audience,
        )?;
    }
    let token = installation_token(&app, destination.installation_id).await?;
    let remote = app.push_remote(&destination.github_full_name, &token);
    let git_ref = push.branch.git_ref();
    let incarnation = state
        .metadata
        .repositories()
        .repository_record(&push.repo_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(gone)?
        .incarnation();
    // Reading the commit may take a while, so the claim and the connection
    // are checked again once it is at hand.
    let (check_state, check_push, claim_token) =
        (state.clone(), push.clone(), claim_token.to_string());
    let expected_oid = target_oid.clone();
    let push_when_current = move |repo: &Path| {
        crate::git::blocking::block_on(ensure_current(
            &check_state,
            &check_push,
            &claim_token,
            audience,
        ))
        .and_then(|()| {
            remote
                .push(repo, &target_oid, &git_ref)
                .map_err(PushFailure::Retry)
        })
    };
    let Some((request, evaluation)) = tested else {
        // Main's own store holds the commit, as it was when the test started.
        let (Some(head), spans) = state
            .metadata
            .repositories()
            .repository_content_source(&incarnation)
            .await
            .map_err(ApiError::from)?
        else {
            return Err(gone());
        };
        let repo = state
            .repository_engine
            .materialize_repository(state, &incarnation, &head, &spans)
            .await?;
        record_setup_check_baseline(state, &app, push).await?;
        return crate::git::blocking::run(move || Ok(push_when_current(repo.as_ref()))).await?;
    };
    let revision = requests
        .request_revision_with_head(&request.id, &evaluation.head_oid)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(gone)?;
    match &evaluation.check_commit_base {
        // A public contribution's check commit is built again from what its
        // evaluation recorded, in a private repository that holds it.
        Some(base) => {
            Box::pin(with_check_commit(
                state,
                &incarnation,
                &request,
                &revision,
                base,
                &expected_oid,
                push_when_current,
            ))
            .await?
        }
        // The revision store holds the head with the history it builds on, the
        // way merge preparation reads a request head.
        None => {
            with_request_revision_store_repo(
                state,
                &incarnation,
                &request,
                &revision,
                move |repo, _| Ok(push_when_current(repo)),
            )
            .await?
        }
    }
}

/// Asks GitHub, right before the connection test's push, which workflow runs
/// it already lists on the setup branch for the commit: they are an earlier
/// test's, and only runs GitHub starts after are this test's. A push that is
/// no longer its test's sends nothing.
async fn record_setup_check_baseline(
    state: &AppState,
    app: &GitHubApp,
    push: &GitHubPush,
) -> Result<(), PushFailure> {
    let destination = &push.destination;
    let target_oid = push.target_oid.as_deref().unwrap_or_default();
    let runs = app
        .branch_workflow_runs(
            destination.installation_id,
            &destination.github_full_name,
            &push.branch.name(),
            target_oid,
        )
        .await?
        .ok_or_else(|| {
            PushFailure::GiveUp(
                "The Scope GitHub App can no longer reach this GitHub repository.".to_string(),
            )
        })?;
    let recorded = state
        .metadata
        .repositories()
        .record_github_setup_check_baseline(
            &push.repo_id,
            &push.id,
            runs.iter().map(|run| run.github_run_id).collect(),
        )
        .await
        .map_err(ApiError::from)?;
    if recorded {
        Ok(())
    } else {
        Err(PushFailure::GiveUp(
            "The connection test this push belonged to has ended.".to_string(),
        ))
    }
}

async fn installation_token(app: &GitHubApp, installation_id: u64) -> Result<String, PushFailure> {
    app.installation_token(installation_id)
        .await?
        .ok_or_else(|| {
            PushFailure::GiveUp(
                "The Scope GitHub App is no longer installed for this repository.".to_string(),
            )
        })
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
