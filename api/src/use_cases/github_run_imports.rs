//! Imports a connected repository's recent workflow runs from GitHub, so its
//! Runs page does not wait for GitHub to report each run again. Connecting
//! queues the import and a maintainer can queue another; a leased background
//! pass reads GitHub's list of runs, newest first, page by page until it has
//! as many as the import asked for or GitHub has no more. Runs are stored the
//! way GitHub's reports store them, so an import never moves a run back and a
//! later report still moves it forward. A failed attempt is tried again after
//! the usual delays and keeps GitHub's answer for the settings page. Before
//! each page the import checks that its claim still holds, so one replaced by
//! a newer import, or by a reconnect, stops storing runs.

use crate::{
    auth::tokens::random_token, error::ApiError, persistence::unix_now,
    repo_events::RepoChangeReason, state::AppState, use_cases::github_workflow_runs,
};
use scope_domain::{
    github_connection::GitHubConnection, github_run_import::GitHubRunImport,
    requests::github_retry_at,
};
use scope_postgres::db::GitHubRunImportOutcome;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const BATCH_SIZE: usize = 2;
/// Comfortably longer than reading ten pages from GitHub takes, so a live
/// import is never raced.
const LEASE_SECS: u64 = 10 * 60;

enum ImportFailure {
    Retry(String),
    GiveUp(String),
    /// The claim lapsed or a newer import replaced this one: nothing more
    /// is stored and nothing is recorded.
    Stale,
}

impl From<ApiError> for ImportFailure {
    fn from(error: ApiError) -> Self {
        Self::Retry(error.into_public_message())
    }
}

/// Runs the imports this process can claim, one at a time. Returns how many
/// it claimed.
pub(crate) async fn import_due_github_runs(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let repositories = state.metadata.repositories();
    let mut claimed = 0;
    while claimed < BATCH_SIZE {
        let claim_token = random_token(
            "github_run_import_claim_",
            "failed to generate run import claim token",
        )?;
        let Some(import) = repositories
            .claim_due_github_run_imports(
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
        run_claimed_import(state, &import, &claim_token, now_unix).await;
    }
    Ok(claimed)
}

/// Imports one claimed import's runs and records how it ended while the claim
/// holds.
async fn run_claimed_import(
    state: &AppState,
    import: &GitHubRunImport,
    claim_token: &str,
    now_unix: u64,
) {
    let imported = import_runs(state, import, claim_token).await;
    let now_unix = unix_now().map_or(now_unix, |finished| finished.max(now_unix));
    let outcome = match imported {
        Ok(imported) => GitHubRunImportOutcome::Succeeded { imported },
        Err(ImportFailure::Retry(error)) => GitHubRunImportOutcome::Failed {
            error,
            retry_at_unix: github_retry_at(import.attempts, now_unix),
        },
        Err(ImportFailure::GiveUp(error)) => GitHubRunImportOutcome::Failed {
            error,
            retry_at_unix: None,
        },
        Err(ImportFailure::Stale) => return,
    };
    if let GitHubRunImportOutcome::Failed { error, .. } = &outcome {
        tracing::warn!(
            repo_id = import.repository_id,
            attempts = import.attempts,
            %error,
            "importing GitHub workflow runs failed"
        );
    }
    let repositories = state.metadata.repositories();
    match repositories
        .finish_github_run_import(&import.repository_id, claim_token, outcome, now_unix)
        .await
    {
        // Maintainers' open settings show how the import ended.
        Ok(true) => match repositories.repository_record(&import.repository_id).await {
            Ok(Some(record)) => {
                state
                    .publish_request_summary_refresh(
                        &record.incarnation(),
                        RepoChangeReason::GitHubConnectionChanged,
                    )
                    .await;
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(
                repo_id = import.repository_id,
                error = %error,
                "could not announce a GitHub run import"
            ),
        },
        // Another process took the import over, or a newer one replaced it.
        Ok(false) => {}
        Err(error) => tracing::warn!(
            repo_id = import.repository_id,
            error = %error,
            "could not record a GitHub run import; its claim will lapse"
        ),
    }
}

/// Reads GitHub's most recent runs until the import has as many as it asked
/// for or GitHub lists no more. Returns how many it stored.
async fn import_runs(
    state: &AppState,
    import: &GitHubRunImport,
    claim_token: &str,
) -> Result<u32, ImportFailure> {
    let app = github_workflow_runs::configured_app(state)?;
    let repositories = state.metadata.repositories();
    let Some(connection) = repositories
        .github_connection(&import.repository_id)
        .await
        .map_err(ApiError::from)?
        .map(|read| read.connection)
        .filter(|connection| connection.is_connected() && import.is_of(connection))
    else {
        return Err(ImportFailure::GiveUp(
            "This repository is no longer connected to that GitHub repository.".to_string(),
        ));
    };
    let mut imported = 0;
    let mut page = 1;
    while import.remaining(imported) > 0 {
        if !repositories
            .github_run_import_claim_holds(&import.repository_id, claim_token)
            .await
            .map_err(ApiError::from)?
        {
            return Err(ImportFailure::Stale);
        }
        let Some(listed) = app
            .recent_workflow_runs(
                connection.installation_id,
                &connection.github_full_name,
                page,
            )
            .await
            // The maintainer sees what GitHub answered.
            .map_err(|error| ImportFailure::Retry(error.into_operator_diagnostic()))?
        else {
            return Err(ImportFailure::GiveUp(
                "The Scope GitHub App can no longer reach this GitHub repository.".to_string(),
            ));
        };
        let wanted = usize::try_from(import.remaining(imported)).unwrap_or(usize::MAX);
        let runs = listed.runs.into_iter().take(wanted).collect::<Vec<_>>();
        store_runs(state, &connection, &runs).await?;
        imported += u32::try_from(runs.len()).unwrap_or(u32::MAX);
        if !listed.more {
            break;
        }
        page += 1;
    }
    Ok(imported)
}

/// Stores a page of runs and tells open Runs pages, so a long import shows
/// its runs as they arrive.
async fn store_runs(
    state: &AppState,
    connection: &GitHubConnection,
    runs: &[scope_domain::github_workflow_runs::GitHubWorkflowRun],
) -> Result<(), ApiError> {
    if runs.is_empty() {
        return Ok(());
    }
    let repositories = state.metadata.repositories();
    for run in runs {
        repositories
            .save_github_workflow_run(
                &connection.repository_id,
                connection.github_repository_id,
                run,
            )
            .await?;
    }
    github_workflow_runs::publish(state, connection).await
}

impl AppState {
    pub(crate) fn start_github_run_imports(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                let pass = async {
                    state.metadata.admin().readiness_check().await?;
                    import_due_github_runs(&state, unix_now()?).await
                };
                if let Err(error) = pass.await {
                    tracing::warn!(
                        error = %error.operator_diagnostic(),
                        "GitHub run import pass failed; retrying"
                    );
                }
                tokio::select! {
                    _ = state.github_run_import_wakeup.notified() => {},
                    _ = tokio::time::sleep(POLL_INTERVAL) => {},
                }
            }
        });
    }
}
