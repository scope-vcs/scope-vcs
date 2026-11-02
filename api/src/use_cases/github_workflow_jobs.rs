use super::github_job_logs;
use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{error::ApiError, persistence::unix_now, state::AppState};
use scope_domain::{
    github_connection::GitHubConnection,
    github_workflow_jobs::{
        GitHubJobsRead, GitHubWorkflowJob, github_jobs_need_read, github_jobs_not_read_yet,
        github_jobs_retry_at,
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
    Retry,
}

pub(crate) async fn queue_jobs_for_delivery(
    state: &AppState,
    github_repository_id: u64,
    run_id: u64,
    run_attempt: u32,
) -> Result<(), ApiError> {
    let repositories = state.metadata.repositories();
    let Some(connection) = repositories
        .github_connection_for_github_repository(github_repository_id)
        .await?
    else {
        return Ok(());
    };
    repositories
        .queue_github_workflow_job_read(
            &connection.repository_id,
            github_repository_id,
            run_id,
            run_attempt,
            unix_now()?,
        )
        .await?;
    state.wakeups.github_job_reads.notify_one();
    Ok(())
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
                run.run_attempt,
                now_unix,
            )
            .await?;
        state.wakeups.github_job_reads.notify_one();
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
            .filter(|connection| connection.is_connected_to(job.github_repository_id));
        let outcome = match &connection {
            Some(connection) => {
                read_queued_jobs(
                    state,
                    connection,
                    job.github_run_id,
                    job.run_attempt,
                    claimed_at,
                )
                .await
            }
            None => Ok(QueuedJobsRead::Missing),
        };
        let finished_at = clock();
        let retry_at = match &outcome {
            Ok(QueuedJobsRead::Missing | QueuedJobsRead::Saved) => None,
            Ok(QueuedJobsRead::Retry) => Some(github_jobs_retry_at(job.attempts, finished_at)),
            Err(error) => {
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
        if let (Ok(QueuedJobsRead::Saved), Some(connection)) = (outcome, &connection) {
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
    run_attempt: u32,
    now_unix: u64,
) -> Result<QueuedJobsRead, ApiError> {
    let repositories = state.metadata.repositories();
    let mut detail = repositories
        .github_workflow_run(
            &connection.repository_id,
            connection.github_repository_id,
            run_id,
        )
        .await?;
    if detail
        .as_ref()
        .is_none_or(|detail| detail.read.run.run_attempt < run_attempt)
    {
        super::github_workflow_runs::read_workflow_run(state, connection, run_id).await?;
        detail = repositories
            .github_workflow_run(
                &connection.repository_id,
                connection.github_repository_id,
                run_id,
            )
            .await?;
    }
    let Some(detail) = detail else {
        return Ok(QueuedJobsRead::Retry);
    };
    if detail.read.run.run_attempt < run_attempt {
        return Ok(QueuedJobsRead::Retry);
    }
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
        return Ok(QueuedJobsRead::Retry);
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

impl BackgroundWork {
    pub(crate) fn start_github_job_reads(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "GitHub job read pass",
                cadence: Cadence::AfterPass {
                    delay: READ_POLL_INTERVAL,
                    wakeup: Some(|wakeups| &wakeups.github_job_reads),
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { retry_github_workflow_job_reads_once(&state, unix_now()?).await },
        );
    }
}

async fn read_run_jobs(
    state: &AppState,
    connection: &GitHubConnection,
    run: &GitHubWorkflowRun,
) -> Result<(), ApiError> {
    let Some(jobs) = state
        .github_app()?
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
    github_job_logs::prefetch(state, &jobs, unix_now()?).await?;
    Ok(())
}

pub(super) async fn publish(
    state: &AppState,
    connection: &GitHubConnection,
    github_run_id: u64,
) -> Result<(), ApiError> {
    if let Some(incarnation) = state
        .repository_incarnation(&connection.repository_id)
        .await?
    {
        state
            .publish_github_workflow_run_change(&incarnation, github_run_id)
            .await;
    }
    Ok(())
}
