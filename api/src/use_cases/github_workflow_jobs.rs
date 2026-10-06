use super::github_workflow_runs::configured_app;
use crate::{error::ApiError, state::AppState};
use scope_domain::{
    github_connection::GitHubConnection,
    github_workflow_jobs::{
        GitHubJobLogState, GitHubJobsRead, GitHubWorkflowJob, github_jobs_need_read,
    },
    github_workflow_runs::GitHubWorkflowRun,
};
use scope_postgres::db::GitHubWorkflowRunDetailRead;

pub(crate) struct GitHubRunJobs {
    pub(crate) jobs: Vec<GitHubWorkflowJob>,
    pub(crate) unavailable: Option<String>,
}

pub(crate) async fn refresh_job_for_delivery(
    state: &AppState,
    github_repository_id: u64,
    job_id: u64,
) -> Result<(), ApiError> {
    let Some(connection) = state
        .metadata
        .repositories()
        .github_connection_for_github_repository(github_repository_id)
        .await?
    else {
        return Ok(());
    };
    let Some(job) = configured_app(state)?
        .workflow_job(
            connection.installation_id,
            &connection.github_full_name,
            job_id,
        )
        .await?
    else {
        return Ok(());
    };
    state
        .metadata
        .repositories()
        .save_github_workflow_jobs(
            &connection.repository_id,
            connection.github_repository_id,
            std::slice::from_ref(&job),
        )
        .await?;
    publish(state, &connection, job.github_run_id).await
}

pub(crate) async fn run_jobs(
    state: &AppState,
    connection: &GitHubConnection,
    detail: &GitHubWorkflowRunDetailRead,
    now_unix: u64,
) -> Result<GitHubRunJobs, ApiError> {
    let run = &detail.read.run;
    let repositories = state.metadata.repositories();
    let stored = || {
        repositories.github_workflow_jobs(
            &connection.repository_id,
            connection.github_repository_id,
            run.github_run_id,
            run.run_attempt,
        )
    };
    let mut jobs = stored().await?;
    let mut unavailable = None;
    let claimed = GitHubJobsRead {
        run_attempt: run.run_attempt,
        read_at_unix: now_unix,
    };
    if connection.is_connected()
        && github_jobs_need_read(run, detail.jobs_read, !jobs.is_empty(), now_unix)
        && repositories
            .replace_github_jobs_read(
                &connection.repository_id,
                run.github_run_id,
                detail.jobs_read,
                Some(claimed),
            )
            .await?
    {
        match read_run_jobs(state, connection, run).await {
            Ok(true) => {
                publish(state, connection, run.github_run_id).await?;
                jobs = stored().await?;
            }
            Ok(false) => {
                unavailable = Some("GitHub no longer reports this run's jobs.".to_string())
            }
            Err(error) => {
                tracing::warn!(
                    run_id = run.github_run_id,
                    error = %error.operator_diagnostic(),
                    "reading a GitHub workflow run's jobs failed"
                );
                repositories
                    .replace_github_jobs_read(
                        &connection.repository_id,
                        run.github_run_id,
                        Some(claimed),
                        detail.jobs_read,
                    )
                    .await?;
                unavailable = Some(
                    "GitHub could not be reached for this run's jobs. Try again shortly."
                        .to_string(),
                );
            }
        }
    }
    Ok(GitHubRunJobs {
        unavailable: unavailable.filter(|_| jobs.is_empty()),
        jobs,
    })
}

async fn read_run_jobs(
    state: &AppState,
    connection: &GitHubConnection,
    run: &GitHubWorkflowRun,
) -> Result<bool, ApiError> {
    let Some(jobs) = configured_app(state)?
        .run_attempt_jobs(
            connection.installation_id,
            &connection.github_full_name,
            run.github_run_id,
            run.run_attempt,
        )
        .await?
    else {
        return Ok(false);
    };
    state
        .metadata
        .repositories()
        .save_github_workflow_jobs(
            &connection.repository_id,
            connection.github_repository_id,
            &jobs,
        )
        .await?;
    Ok(true)
}

pub(crate) async fn job_log(
    state: &AppState,
    connection: &GitHubConnection,
    job: &GitHubWorkflowJob,
    now_unix: u64,
) -> Result<GitHubJobLogState, ApiError> {
    if !job.is_completed() {
        return Err(ApiError::conflict(
            "This job's log is available once the job finishes.",
        ));
    }
    let repositories = state.metadata.repositories();
    if let Some(log) = repositories
        .github_workflow_job_log(job.github_job_id)
        .await?
    {
        return Ok(log);
    }
    if !connection.is_connected() {
        return Err(ApiError::conflict(
            "This repository is no longer connected to GitHub, so the log cannot be read.",
        ));
    }
    let log = configured_app(state)?
        .job_log(
            connection.installation_id,
            &connection.github_full_name,
            job.github_job_id,
        )
        .await?;
    repositories
        .save_github_workflow_job_log(job.github_job_id, &log, now_unix)
        .await?;
    Ok(log)
}

async fn publish(
    state: &AppState,
    connection: &GitHubConnection,
    github_run_id: u64,
) -> Result<(), ApiError> {
    if let Some(record) = state
        .metadata
        .repositories()
        .repository_record(&connection.repository_id)
        .await?
    {
        state
            .publish_github_workflow_run_change(&record.incarnation(), github_run_id)
            .await;
    }
    Ok(())
}
