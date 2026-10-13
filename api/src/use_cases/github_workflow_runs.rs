use crate::{error::ApiError, state::AppState};
use scope_domain::{
    github_connection::GitHubConnection,
    requests::{GitHubBranch, github_retry_at},
};
use scope_postgres::db::GitHubWorkflowRunReadJob;

const READ_BATCH_SIZE: u64 = 20;
const READ_LEASE_SECS: u64 = 5 * 60;

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
        Ok(()) => Ok(repositories
            .finish_github_workflow_run_read(&job, None)
            .await?),
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
            .filter(|connection| connection.is_connected_to(job.github_repository_id));
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

async fn read_workflow_run(
    state: &AppState,
    connection: &GitHubConnection,
    run_id: u64,
) -> Result<(), ApiError> {
    let Some(run) = state
        .github_app()?
        .workflow_run(
            connection.installation_id,
            &connection.github_full_name,
            run_id,
        )
        .await?
    else {
        return Ok(());
    };
    save_runs(state, connection, &[run]).await
}

pub(crate) async fn refresh_branch_workflow_runs(
    state: &AppState,
    connection: &GitHubConnection,
    branch: &GitHubBranch,
    commit_oid: &str,
) -> Result<(), ApiError> {
    let Some(runs) = state
        .github_app()?
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
    save_runs(state, connection, &runs).await
}

async fn save_runs(
    state: &AppState,
    connection: &GitHubConnection,
    runs: &[scope_domain::github_workflow_runs::GitHubWorkflowRun],
) -> Result<(), ApiError> {
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
    if runs.is_empty() {
        return Ok(());
    }
    publish(state, connection).await
}

pub(crate) async fn publish(
    state: &AppState,
    connection: &GitHubConnection,
) -> Result<(), ApiError> {
    if let Some(incarnation) = state
        .repository_incarnation(&connection.repository_id)
        .await?
    {
        state
            .publish_github_workflow_runs_change(&incarnation)
            .await;
    }
    Ok(())
}
