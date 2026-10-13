use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{
    auth::tokens::random_token,
    error::ApiError,
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::{
        github_pushes::{JobFailure, job_outcome},
        github_workflow_runs,
    },
};
use scope_domain::{github_run_import::GitHubRunImport, requests::github_retry_at};
use scope_postgres::db::GitHubJobOutcome;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const BATCH_SIZE: usize = 2;
const LEASE_SECS: u64 = 10 * 60;

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

async fn run_claimed_import(
    state: &AppState,
    import: &GitHubRunImport,
    claim_token: &str,
    now_unix: u64,
) {
    let imported = import_runs(state, import, claim_token).await;
    let now_unix = unix_now().map_or(now_unix, |finished| finished.max(now_unix));
    let Some(outcome) = job_outcome(imported, github_retry_at(import.attempts, now_unix)) else {
        return;
    };
    if let GitHubJobOutcome::Failed { error, .. } = &outcome {
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
        Ok(true) => match state.repository_incarnation(&import.repository_id).await {
            Ok(Some(incarnation)) => {
                state
                    .publish_request_summary_refresh(
                        &incarnation,
                        RepoChangeReason::GitHubConnectionChanged,
                    )
                    .await;
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(
                repo_id = import.repository_id,
                error = %error.operator_diagnostic(),
                "could not announce a GitHub run import"
            ),
        },
        Ok(false) => {}
        Err(error) => tracing::warn!(
            repo_id = import.repository_id,
            error = %error,
            "could not record a GitHub run import; its claim will lapse"
        ),
    }
}

async fn import_runs(
    state: &AppState,
    import: &GitHubRunImport,
    claim_token: &str,
) -> Result<u32, JobFailure> {
    let app = state.github_app()?;
    let repositories = state.metadata.repositories();
    let Some(connection) = repositories
        .github_connection(&import.repository_id)
        .await?
        .filter(|connection| connection.is_connected_to(import.github_repository_id))
    else {
        return Err(JobFailure::GiveUp(
            "This repository is no longer connected to that GitHub repository.".to_string(),
        ));
    };
    let mut imported = 0;
    let mut page = 1;
    while import.remaining(imported) > 0 {
        let Some(listed) = app
            .recent_workflow_runs(
                connection.installation_id,
                &connection.github_full_name,
                page,
            )
            .await
            .map_err(|error| JobFailure::Retry(error.into_operator_diagnostic()))?
        else {
            return Err(JobFailure::GiveUp(
                "The Scope GitHub App can no longer reach this GitHub repository.".to_string(),
            ));
        };
        let wanted = usize::try_from(import.remaining(imported)).unwrap_or(usize::MAX);
        let runs = listed.runs.into_iter().take(wanted).collect::<Vec<_>>();
        if !repositories
            .store_github_run_import_page(&import.repository_id, claim_token, &runs)
            .await?
        {
            return Err(JobFailure::Stale);
        }
        if !runs.is_empty() {
            github_workflow_runs::publish(state, &connection).await?;
        }
        imported += u32::try_from(runs.len()).unwrap_or(u32::MAX);
        if !listed.more {
            break;
        }
        page += 1;
    }
    Ok(imported)
}

impl BackgroundWork {
    pub(crate) fn start_github_run_imports(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "GitHub run import pass",
                cadence: Cadence::AfterPass {
                    delay: POLL_INTERVAL,
                    wakeup: Some(|wakeups| &wakeups.github_run_imports),
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { import_due_github_runs(&state, unix_now()?).await },
        );
    }
}
