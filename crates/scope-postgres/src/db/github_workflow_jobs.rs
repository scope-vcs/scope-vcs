use super::{
    RepositoryStore,
    entities::{decode_enum, encode_enum},
    github_read_queue::GitHubReadQueue,
    integer_columns::{
        i32_to_u32, i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, u32_to_i32, u64_to_i64,
    },
};
use crate::error::PostgresError;
use scope_domain::github_workflow_jobs::{GitHubJobLog, GitHubJobLogState, GitHubWorkflowJob};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};

#[derive(FromQueryResult)]
struct JobRow {
    github_job_id: i64,
    github_run_id: i64,
    run_attempt: i32,
    name: String,
    status: String,
    conclusion: Option<String>,
    started_at_unix: Option<i64>,
    completed_at_unix: Option<i64>,
    html_url: String,
    steps: serde_json::Value,
}

#[derive(FromQueryResult)]
struct LogRow {
    log_text: Option<String>,
    truncated: bool,
}

const SELECT_JOB: &str = "github_job_id, github_run_id, run_attempt, name, status, conclusion,
    started_at_unix, completed_at_unix, html_url, steps";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowJobReadJob {
    pub run_attempt: u32,
    pub read_id: u64,
    pub repo_id: String,
    pub github_repository_id: u64,
    pub github_run_id: u64,
    pub attempts: u32,
    pub generation: u64,
}

#[derive(FromQueryResult)]
struct JobReadRow {
    run_attempt: i32,
    read_id: i64,
    repo_id: String,
    github_repository_id: i64,
    github_run_id: i64,
    attempts: i32,
    generation: i64,
}

const JOB_READS: GitHubReadQueue = GitHubReadQueue {
    table: "scope_github_workflow_job_reads",
    key: &["repo_id", "github_repository_id", "github_run_id"],
    joined: "",
    joined_on: "",
    returning: "read.read_id, read.repo_id, read.github_repository_id, read.github_run_id, \
        read.attempts, read.generation, read.run_attempt",
    label: "GitHub jobs read",
};

impl RepositoryStore {
    pub async fn queue_github_workflow_job_read(
        &self,
        repo_id: &str,
        github_repository_id: u64,
        github_run_id: u64,
        run_attempt: u32,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        self.db.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_github_workflow_job_reads (repo_id, github_repository_id, github_run_id, attempts, generation, next_attempt_at_unix, run_attempt)
             VALUES ($1, $2, $3, 0, 0, $4, $5)
             ON CONFLICT (repo_id, github_repository_id, github_run_id)
             DO UPDATE SET generation = scope_github_workflow_job_reads.generation + 1,
                           run_attempt = greatest(scope_github_workflow_job_reads.run_attempt, EXCLUDED.run_attempt)",
            [repo_id.into(), u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
             u64_to_i64(github_run_id, "GitHub workflow run id")?.into(),
             u64_to_i64(now_unix, "GitHub jobs read time")?.into(),
             u32_to_i32(run_attempt, "GitHub run attempt")?.into()],
        )).await?;
        Ok(())
    }

    pub async fn claim_due_github_workflow_job_reads(
        &self,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubWorkflowJobReadJob>, PostgresError> {
        JOB_READS
            .claim_due::<JobReadRow>(self.db.as_ref(), now_unix, lease_until_unix, limit)
            .await?
            .into_iter()
            .map(|row| {
                Ok(GitHubWorkflowJobReadJob {
                    run_attempt: i32_to_u32(row.run_attempt, "GitHub run attempt")?,
                    read_id: i64_to_u64(row.read_id, "GitHub jobs read id")?,
                    repo_id: row.repo_id,
                    github_repository_id: i64_to_u64(
                        row.github_repository_id,
                        "GitHub repository id",
                    )?,
                    github_run_id: i64_to_u64(row.github_run_id, "GitHub workflow run id")?,
                    attempts: i32_to_u32(row.attempts, "GitHub jobs read attempts")?,
                    generation: i64_to_u64(row.generation, "GitHub jobs read generation")?,
                })
            })
            .collect()
    }

    pub async fn finish_github_workflow_job_read(
        &self,
        job: &GitHubWorkflowJobReadJob,
        retry_at_unix: Option<u64>,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let claim = vec![
            job.repo_id.clone().into(),
            u64_to_i64(job.github_repository_id, "GitHub repository id")?.into(),
            u64_to_i64(job.github_run_id, "GitHub workflow run id")?.into(),
            u32_to_i32(job.attempts, "GitHub jobs read attempts")?.into(),
            u64_to_i64(job.read_id, "GitHub jobs read id")?.into(),
        ];
        JOB_READS
            .finish_generation(
                self.db.as_ref(),
                claim,
                job.generation,
                retry_at_unix,
                now_unix,
            )
            .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "save_github_workflow_jobs"))]
    pub async fn save_github_workflow_jobs(
        &self,
        repo_id: &str,
        github_repository_id: u64,
        jobs: &[GitHubWorkflowJob],
    ) -> Result<(), PostgresError> {
        for job in jobs {
            save_job(self.db.as_ref(), repo_id, github_repository_id, job).await?;
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_workflow_jobs"))]
    pub async fn github_workflow_jobs(
        &self,
        repo_id: &str,
        github_repository_id: u64,
        github_run_id: u64,
        run_attempt: u32,
    ) -> Result<Vec<GitHubWorkflowJob>, PostgresError> {
        JobRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {SELECT_JOB} FROM scope_github_workflow_jobs
                  WHERE repo_id = $1 AND github_repository_id = $2
                    AND github_run_id = $3 AND run_attempt = $4
                  ORDER BY github_job_id"
            ),
            [
                repo_id.into(),
                u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
                u64_to_i64(github_run_id, "GitHub workflow run id")?.into(),
                u32_to_i32(run_attempt, "GitHub run attempt")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await?
        .into_iter()
        .map(JobRow::into_domain)
        .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_workflow_job"))]
    pub async fn github_workflow_job(
        &self,
        repo_id: &str,
        github_repository_id: u64,
        github_run_id: u64,
        github_job_id: u64,
    ) -> Result<Option<GitHubWorkflowJob>, PostgresError> {
        JobRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {SELECT_JOB} FROM scope_github_workflow_jobs
                  WHERE repo_id = $1 AND github_repository_id = $2
                    AND github_run_id = $3 AND github_job_id = $4"
            ),
            [
                repo_id.into(),
                u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
                u64_to_i64(github_run_id, "GitHub workflow run id")?.into(),
                u64_to_i64(github_job_id, "GitHub job id")?.into(),
            ],
        ))
        .one(self.db.as_ref())
        .await?
        .map(JobRow::into_domain)
        .transpose()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_workflow_job_log"))]
    pub async fn github_workflow_job_log(
        &self,
        github_job_id: u64,
    ) -> Result<Option<GitHubJobLogState>, PostgresError> {
        Ok(LogRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT log_text, truncated FROM scope_github_workflow_job_logs
              WHERE github_job_id = $1",
            [u64_to_i64(github_job_id, "GitHub job id")?.into()],
        ))
        .one(self.db.as_ref())
        .await?
        .map(|row| match row.log_text {
            Some(text) => GitHubJobLogState::Kept(GitHubJobLog {
                text,
                truncated: row.truncated,
            }),
            None => GitHubJobLogState::Expired,
        }))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "save_github_workflow_job_log"))]
    pub async fn save_github_workflow_job_log(
        &self,
        github_job_id: u64,
        log: &GitHubJobLogState,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let (text, truncated) = match log {
            GitHubJobLogState::Kept(log) => (Some(log.text.clone()), log.truncated),
            GitHubJobLogState::Expired => (None, false),
        };
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_workflow_job_logs
                    (github_job_id, log_text, truncated, stored_at_unix)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT DO NOTHING",
                [
                    u64_to_i64(github_job_id, "GitHub job id")?.into(),
                    text.into(),
                    truncated.into(),
                    u64_to_i64(now_unix, "GitHub job log time")?.into(),
                ],
            ))
            .await?;
        Ok(())
    }
}

async fn save_job<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    github_repository_id: u64,
    job: &GitHubWorkflowJob,
) -> Result<(), PostgresError> {
    let progress = job.progress();
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_github_workflow_jobs (github_job_id, repo_id, github_repository_id,
                github_run_id, run_attempt, name, status, conclusion, started_at_unix,
                completed_at_unix, html_url, steps, stage, steps_completed, steps_started)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT (github_job_id) DO UPDATE SET
                repo_id = EXCLUDED.repo_id,
                github_repository_id = EXCLUDED.github_repository_id,
                github_run_id = EXCLUDED.github_run_id, run_attempt = EXCLUDED.run_attempt,
                name = EXCLUDED.name, status = EXCLUDED.status,
                conclusion = EXCLUDED.conclusion,
                started_at_unix = EXCLUDED.started_at_unix,
                completed_at_unix = EXCLUDED.completed_at_unix,
                html_url = EXCLUDED.html_url, steps = EXCLUDED.steps,
                stage = EXCLUDED.stage, steps_completed = EXCLUDED.steps_completed,
                steps_started = EXCLUDED.steps_started
              WHERE (scope_github_workflow_jobs.stage, scope_github_workflow_jobs.steps_completed,
                     scope_github_workflow_jobs.steps_started)
                    <= (EXCLUDED.stage, EXCLUDED.steps_completed, EXCLUDED.steps_started)
                AND (scope_github_workflow_jobs.repo_id = EXCLUDED.repo_id
                     OR EXISTS (
                         SELECT 1 FROM scope_github_connections connection
                          WHERE connection.repo_id = EXCLUDED.repo_id
                            AND connection.github_repository_id = EXCLUDED.github_repository_id
                            AND connection.status = 'Connected'))",
        [
            u64_to_i64(job.github_job_id, "GitHub job id")?.into(),
            repo_id.into(),
            u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
            u64_to_i64(job.github_run_id, "GitHub workflow run id")?.into(),
            u32_to_i32(job.run_attempt, "GitHub run attempt")?.into(),
            job.name.clone().into(),
            encode_enum(job.status)?.into(),
            job.conclusion.map(encode_enum).transpose()?.into(),
            optional_u64_to_i64(job.started_at_unix, "GitHub job start")?.into(),
            optional_u64_to_i64(job.completed_at_unix, "GitHub job completion")?.into(),
            job.html_url.clone().into(),
            serde_json::to_value(&job.steps)
                .map_err(PostgresError::internal)?
                .into(),
            i16::from(progress.stage).into(),
            u32_to_i32(progress.steps_completed, "GitHub job steps")?.into(),
            u32_to_i32(progress.steps_started, "GitHub job steps")?.into(),
        ],
    ))
    .await?;
    Ok(())
}

impl JobRow {
    fn into_domain(self) -> Result<GitHubWorkflowJob, PostgresError> {
        Ok(GitHubWorkflowJob {
            github_job_id: i64_to_u64(self.github_job_id, "GitHub job id")?,
            github_run_id: i64_to_u64(self.github_run_id, "GitHub workflow run id")?,
            run_attempt: i32_to_u32(self.run_attempt, "GitHub run attempt")?,
            name: self.name,
            status: decode_enum(self.status)?,
            conclusion: self.conclusion.map(decode_enum).transpose()?,
            started_at_unix: optional_i64_to_u64(self.started_at_unix, "GitHub job start")?,
            completed_at_unix: optional_i64_to_u64(
                self.completed_at_unix,
                "GitHub job completion",
            )?,
            html_url: self.html_url,
            steps: serde_json::from_value(self.steps).map_err(PostgresError::internal)?,
        })
    }
}

#[cfg(test)]
mod tests;
