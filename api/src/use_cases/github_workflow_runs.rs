//! Reads the workflow runs GitHub Actions reports for connected repositories,
//! which their Runs pages list. A `workflow_run` delivery names the run to read
//! again; GitHub's API answers what it is now, so a late or repeated delivery
//! cannot leave an older state behind. The read is kept as pending before
//! GitHub is asked, so one that fails is tried again by a background pass
//! instead of being lost with the delivery.

use crate::{error::ApiError, state::AppState};
use scope_domain::{
    github_connection::GitHubConnection,
    requests::{GitHubBranch, github_retry_at},
};
use scope_postgres::db::GitHubWorkflowRunReadJob;

const READ_BATCH_SIZE: u64 = 20;
/// When a claimed read is tried again if its process never reports back.
const READ_LEASE_SECS: u64 = 5 * 60;

/// A delivery said a workflow run changed. Repositories that are not
/// connected are acknowledged and ignored. Fails only when the read could not
/// be kept for later; a read GitHub does not answer now is tried again.
pub(crate) async fn refresh_workflow_run_for_delivery(
    state: &AppState,
    github_repository_id: u64,
    run_id: u64,
    now_unix: u64,
) -> Result<(), ApiError> {
    let repositories = state.metadata.repositories();
    let Some(connection) = repositories
        .github_connection_for_github_repository(github_repository_id)
        .await?
    else {
        return Ok(());
    };
    repositories
        .queue_github_workflow_run_read(
            &connection.repository_id,
            github_repository_id,
            run_id,
            now_unix,
        )
        .await?;
    let job = GitHubWorkflowRunReadJob {
        repo_id: connection.repository_id.clone(),
        github_repository_id,
        github_run_id: run_id,
        attempts: 0,
    };
    match read_workflow_run(state, &connection, run_id).await {
        Ok(()) => repositories
            .finish_github_workflow_run_read(&job, None)
            .await
            .map_err(ApiError::from),
        Err(error) => {
            tracing::warn!(
                github_repository_id,
                run_id,
                error = %error.operator_diagnostic(),
                "reading a GitHub workflow run for a delivery failed; it is read again later"
            );
            Ok(())
        }
    }
}

/// One pass over pending reads that are due. Each is read again, dropped
/// once its repository is no longer connected to the GitHub repository that
/// reported it, and given up after the usual retries. Returns how many were
/// answered.
pub(crate) async fn retry_github_workflow_run_reads_once(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let repositories = state.metadata.repositories();
    let jobs = repositories
        .claim_due_github_workflow_run_reads(
            now_unix,
            now_unix.saturating_add(READ_LEASE_SECS),
            READ_BATCH_SIZE,
        )
        .await?;
    let mut answered = 0;
    for job in jobs {
        let connection = repositories
            .github_connection(&job.repo_id)
            .await?
            .map(|read| read.connection)
            .filter(|connection| {
                connection.is_connected()
                    && connection.github_repository_id == job.github_repository_id
            });
        let retry_at = match connection {
            None => None,
            Some(connection) => {
                match read_workflow_run(state, &connection, job.github_run_id).await {
                    Ok(()) => {
                        answered += 1;
                        None
                    }
                    Err(error) => {
                        let retry_at = github_retry_at(job.attempts, now_unix);
                        tracing::warn!(
                            run_id = job.github_run_id,
                            attempts = job.attempts,
                            gives_up = retry_at.is_none(),
                            error = %error.operator_diagnostic(),
                            "reading a GitHub workflow run failed"
                        );
                        retry_at
                    }
                }
            }
        };
        repositories
            .finish_github_workflow_run_read(&job, retry_at)
            .await?;
    }
    Ok(answered)
}

/// Stores what GitHub reports for the run now. A run GitHub no longer has is
/// an answer too.
async fn read_workflow_run(
    state: &AppState,
    connection: &GitHubConnection,
    run_id: u64,
) -> Result<(), ApiError> {
    let Some(run) = configured_app(state)?
        .workflow_run(
            connection.installation_id,
            &connection.github_full_name,
            run_id,
        )
        .await?
    else {
        return Ok(());
    };
    state
        .metadata
        .repositories()
        .save_github_workflow_run(
            &connection.repository_id,
            connection.github_repository_id,
            &run,
        )
        .await?;
    publish(state, connection).await
}

/// Reads every workflow run GitHub started on one of Scope's branches for a
/// commit. A connection test reads them itself, so it does not depend on
/// deliveries arriving.
pub(crate) async fn refresh_branch_workflow_runs(
    state: &AppState,
    connection: &GitHubConnection,
    branch: &GitHubBranch,
    commit_oid: &str,
) -> Result<(), ApiError> {
    let Some(runs) = configured_app(state)?
        .branch_workflow_runs(
            connection.installation_id,
            &connection.github_full_name,
            &branch.name(),
            commit_oid,
        )
        .await?
    else {
        return Ok(());
    };
    let repositories = state.metadata.repositories();
    for run in &runs {
        repositories
            .save_github_workflow_run(
                &connection.repository_id,
                connection.github_repository_id,
                run,
            )
            .await?;
    }
    if !runs.is_empty() {
        publish(state, connection).await?;
    }
    Ok(())
}

async fn publish(state: &AppState, connection: &GitHubConnection) -> Result<(), ApiError> {
    if let Some(record) = state
        .metadata
        .repositories()
        .repository_record(&connection.repository_id)
        .await?
    {
        state
            .publish_github_workflow_runs_change(&record.incarnation())
            .await;
    }
    Ok(())
}

fn configured_app(state: &AppState) -> Result<&crate::github::GitHubApp, ApiError> {
    state
        .github
        .as_deref()
        .ok_or_else(|| ApiError::not_found("GitHub is not configured on this server"))
}
