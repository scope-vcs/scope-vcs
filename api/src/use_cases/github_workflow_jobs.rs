use super::github_workflow_runs::configured_app;
use crate::{error::ApiError, state::AppState};
use scope_domain::{
    github_connection::GitHubConnection,
    github_workflow_jobs::{
        GitHubJobLogState, GitHubJobsRead, GitHubWorkflowJob, github_jobs_need_read,
        github_jobs_not_read_yet, github_jobs_retry_at,
    },
    github_workflow_runs::GitHubWorkflowRun,
};
use scope_postgres::db::GitHubWorkflowRunDetailRead;
use std::time::Duration;

const READ_BATCH_SIZE: u64 = 20;
const READ_LEASE_SECS: u64 = 5 * 60;
const READ_POLL_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct GitHubRunJobs {
    pub(crate) jobs: Vec<GitHubWorkflowJob>,
    pub(crate) not_read_yet: bool,
}

enum QueuedJobsRead {
    Saved,
    Missing,
    ClaimLost,
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
    let jobs = repositories
        .github_workflow_jobs(
            &connection.repository_id,
            connection.github_repository_id,
            run.github_run_id,
            run.run_attempt,
        )
        .await?;
    let queued = connection.is_connected()
        && github_jobs_need_read(run, detail.jobs_read, !jobs.is_empty(), now_unix);
    if queued {
        repositories
            .queue_github_workflow_job_read(
                &connection.repository_id,
                connection.github_repository_id,
                run.github_run_id,
                now_unix,
            )
            .await?;
        state.github_job_read_wakeup.notify_one();
    }
    Ok(GitHubRunJobs {
        not_read_yet: github_jobs_not_read_yet(
            run,
            detail.jobs_read,
            !jobs.is_empty(),
            connection.is_connected(),
            detail.jobs_read_queued || queued,
        ),
        jobs,
    })
}

pub(crate) async fn retry_github_workflow_job_reads_once(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let repositories = state.metadata.repositories();
    let started = std::time::Instant::now();
    let clock = || now_unix.saturating_add(started.elapsed().as_secs());
    let mut answered = 0;
    for _ in 0..READ_BATCH_SIZE {
        let claimed_at = clock();
        let Some(job) = repositories
            .claim_due_github_workflow_job_reads(
                claimed_at,
                claimed_at.saturating_add(READ_LEASE_SECS),
                1,
            )
            .await?
            .pop()
        else {
            break;
        };
        let connection = repositories
            .github_connection(&job.repo_id)
            .await?
            .map(|read| read.connection)
            .filter(|connection| {
                connection.is_connected()
                    && connection.github_repository_id == job.github_repository_id
            });
        let mut publish_for = None;
        let outcome = match connection.as_ref() {
            Some(connection) => {
                Some(read_queued_jobs(state, connection, job.github_run_id, claimed_at).await)
            }
            None => None,
        };
        let finished_at = clock();
        let retry_at = match outcome {
            None | Some(Ok(QueuedJobsRead::Missing)) => None,
            Some(Ok(QueuedJobsRead::Saved)) => {
                publish_for = connection.as_ref();
                None
            }
            Some(Ok(QueuedJobsRead::ClaimLost)) => {
                Some(github_jobs_retry_at(job.attempts, finished_at))
            }
            Some(Err(error)) => {
                tracing::warn!(
                    run_id = job.github_run_id,
                    attempts = job.attempts,
                    error = %error.operator_diagnostic(),
                    "reading a GitHub workflow run's jobs failed"
                );
                Some(github_jobs_retry_at(job.attempts, finished_at))
            }
        };
        repositories
            .finish_github_workflow_job_read(&job, retry_at, finished_at)
            .await?;
        if let Some(connection) = publish_for {
            publish(state, connection, job.github_run_id).await?;
            answered += 1;
        }
    }
    Ok(answered)
}

async fn read_queued_jobs(
    state: &AppState,
    connection: &GitHubConnection,
    run_id: u64,
    now_unix: u64,
) -> Result<QueuedJobsRead, ApiError> {
    let repositories = state.metadata.repositories();
    let Some(detail) = repositories
        .github_workflow_run(
            &connection.repository_id,
            connection.github_repository_id,
            run_id,
        )
        .await?
    else {
        return Ok(QueuedJobsRead::Missing);
    };
    let claimed = GitHubJobsRead {
        run_attempt: detail.read.run.run_attempt,
        read_at_unix: now_unix,
    };
    if !repositories
        .replace_github_jobs_read(
            &connection.repository_id,
            run_id,
            detail.jobs_read,
            Some(claimed),
        )
        .await?
    {
        return Ok(QueuedJobsRead::ClaimLost);
    }
    match read_run_jobs(state, connection, &detail.read.run).await {
        Ok(()) => Ok(QueuedJobsRead::Saved),
        Err(error) => {
            repositories
                .replace_github_jobs_read(
                    &connection.repository_id,
                    run_id,
                    Some(claimed),
                    detail.jobs_read,
                )
                .await?;
            Err(error)
        }
    }
}

impl AppState {
    pub(crate) fn start_github_job_reads(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                let pass = async {
                    state.metadata.admin().readiness_check().await?;
                    retry_github_workflow_job_reads_once(&state, crate::persistence::unix_now()?)
                        .await
                };
                if let Err(error) = pass.await {
                    tracing::warn!(error = %error.operator_diagnostic(), "GitHub job read pass failed; retrying");
                }
                tokio::select! {
                    _ = state.github_job_read_wakeup.notified() => {},
                    _ = tokio::time::sleep(READ_POLL_INTERVAL) => {},
                }
            }
        });
    }
}

async fn read_run_jobs(
    state: &AppState,
    connection: &GitHubConnection,
    run: &GitHubWorkflowRun,
) -> Result<(), ApiError> {
    let Some(jobs) = configured_app(state)?
        .run_attempt_jobs(
            connection.installation_id,
            &connection.github_full_name,
            run.github_run_id,
            run.run_attempt,
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
            &jobs,
        )
        .await?;
    Ok(())
}

pub(crate) enum GitHubJobLogRead {
    NotRun,
    Read(GitHubJobLogState),
}

pub(crate) async fn job_log(
    state: &AppState,
    connection: &GitHubConnection,
    job: &GitHubWorkflowJob,
    now_unix: u64,
) -> Result<GitHubJobLogRead, ApiError> {
    if !job.is_completed() {
        return Err(ApiError::conflict(
            "This job's log is available once the job finishes.",
        ));
    }
    if !job.ran() {
        return Ok(GitHubJobLogRead::NotRun);
    }
    let repositories = state.metadata.repositories();
    if let Some(log) = repositories
        .github_workflow_job_log(job.github_job_id)
        .await?
    {
        return Ok(GitHubJobLogRead::Read(log));
    }
    if !connection.is_connected() {
        return Err(ApiError::conflict(
            "This repository is no longer connected to GitHub, so the log cannot be read.",
        ));
    }
    let log = match configured_app(state)?
        .job_log(
            connection.installation_id,
            &connection.github_full_name,
            job.github_job_id,
        )
        .await?
    {
        Some(log) => log,
        None if job.missing_log_is_final(now_unix) => GitHubJobLogState::Expired,
        None => {
            return Err(ApiError::conflict(
                "GitHub hasn't published this job's log yet. Try again shortly.",
            ));
        }
    };
    repositories
        .save_github_workflow_job_log(job.github_job_id, &log, now_unix)
        .await?;
    Ok(GitHubJobLogRead::Read(log))
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
