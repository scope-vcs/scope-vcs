use super::{github_workflow_jobs::publish, github_workflow_runs::configured_app};
use crate::{error::ApiError, persistence::unix_now, state::AppState};
use scope_domain::{
    github_connection::GitHubConnection,
    github_workflow_jobs::{GitHubJobLogState, GitHubWorkflowJob, github_jobs_retry_at},
};
use scope_postgres::db::GitHubJobLogReadJob;
use std::time::Duration;

const READ_BATCH_SIZE: u64 = 10;
const READ_LEASE_SECS: u64 = 5 * 60;
const POLL_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) enum GitHubJobLogRead {
    NotRun,
    Pending,
    Read(GitHubJobLogState),
}

enum LogReadOutcome {
    Settled,
    Unpublished,
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
    if let Some(log) = state
        .metadata
        .repositories()
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
    queue(state, job.github_job_id, now_unix).await?;
    Ok(GitHubJobLogRead::Pending)
}

pub(super) async fn prefetch(
    state: &AppState,
    jobs: &[GitHubWorkflowJob],
    now_unix: u64,
) -> Result<(), ApiError> {
    for job in jobs.iter().filter(|job| job.log_is_prefetched()) {
        queue(state, job.github_job_id, now_unix).await?;
    }
    Ok(())
}

async fn queue(state: &AppState, github_job_id: u64, now_unix: u64) -> Result<(), ApiError> {
    state
        .metadata
        .repositories()
        .queue_github_job_log_read(github_job_id, now_unix)
        .await?;
    state.github_job_log_wakeup.notify_one();
    Ok(())
}

pub(crate) async fn read_due_github_job_logs_once(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let repositories = state.metadata.repositories();
    let started = std::time::Instant::now();
    let clock = || now_unix.saturating_add(started.elapsed().as_secs());
    let mut claimed = 0;
    while claimed < READ_BATCH_SIZE as usize {
        let claimed_at = clock();
        let Some(read) = repositories
            .claim_due_github_job_log_reads(
                claimed_at,
                claimed_at.saturating_add(READ_LEASE_SECS),
                1,
            )
            .await?
            .pop()
        else {
            break;
        };
        claimed += 1;
        let outcome = read_log(state, &read, claimed_at).await;
        let finished_at = clock();
        let retry_at = match outcome {
            Ok(LogReadOutcome::Settled) => None,
            Ok(LogReadOutcome::Unpublished) => {
                Some(github_jobs_retry_at(read.attempts, finished_at))
            }
            Err(error) => {
                tracing::warn!(
                    job_id = read.github_job_id,
                    attempts = read.attempts,
                    error = %error.operator_diagnostic(),
                    "reading a GitHub job's log failed"
                );
                Some(github_jobs_retry_at(read.attempts, finished_at))
            }
        };
        repositories
            .finish_github_job_log_read(&read, retry_at)
            .await?;
    }
    Ok(claimed)
}

async fn read_log(
    state: &AppState,
    read: &GitHubJobLogReadJob,
    now_unix: u64,
) -> Result<LogReadOutcome, ApiError> {
    let repositories = state.metadata.repositories();
    let Some(connection) = repositories
        .github_connection(&read.repo_id)
        .await?
        .map(|read| read.connection)
        .filter(|connection| connection.is_connected())
        .filter(|connection| connection.github_repository_id == read.github_repository_id)
    else {
        return Ok(LogReadOutcome::Settled);
    };
    let Some(job) = repositories
        .github_workflow_job(
            &read.repo_id,
            read.github_repository_id,
            read.github_run_id,
            read.github_job_id,
        )
        .await?
        .filter(|job| job.is_completed() && job.ran())
    else {
        return Ok(LogReadOutcome::Settled);
    };
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
        None => return Ok(LogReadOutcome::Unpublished),
    };
    repositories
        .save_github_workflow_job_log(job.github_job_id, &log, now_unix)
        .await?;
    publish(state, &connection, job.github_run_id).await?;
    Ok(LogReadOutcome::Settled)
}

impl AppState {
    pub(crate) fn start_github_job_log_reads(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                let pass = async {
                    state.metadata.admin().readiness_check().await?;
                    read_due_github_job_logs_once(&state, unix_now()?).await
                };
                match pass.await {
                    Ok(claimed) if claimed as u64 == READ_BATCH_SIZE => continue,
                    Ok(_) => {}
                    Err(error) => tracing::warn!(
                        error = %error.operator_diagnostic(),
                        "GitHub job log read pass failed; retrying"
                    ),
                }
                tokio::select! {
                    _ = state.github_job_log_wakeup.notified() => {},
                    _ = tokio::time::sleep(POLL_INTERVAL) => {},
                }
            }
        });
    }
}
