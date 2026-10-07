use super::{
    RepositoryStore,
    entities::{decode_enum, encode_enum},
    integer_columns::{
        i32_to_u32, i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, u32_to_i32, u64_to_i64,
    },
};
use crate::error::PostgresError;
use scope_domain::{
    github_workflow_jobs::{
        GitHubJobLog, GitHubJobLogState, GitHubWorkflowJob, GitHubWorkflowStep,
    },
    requests::{GitHubCheckConclusion, GitHubCheckStatus},
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, Value};
use serde::{Deserialize, Serialize};

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

#[derive(Deserialize, Serialize)]
struct StoredStep {
    number: u32,
    name: String,
    status: GitHubCheckStatus,
    conclusion: Option<GitHubCheckConclusion>,
    started_at_unix: Option<u64>,
    completed_at_unix: Option<u64>,
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
    pub read_id: u64,
    pub repo_id: String,
    pub github_repository_id: u64,
    pub github_run_id: u64,
    pub attempts: u32,
    pub generation: u64,
}

#[derive(FromQueryResult)]
struct JobReadRow {
    read_id: i64,
    repo_id: String,
    github_repository_id: i64,
    github_run_id: i64,
    attempts: i32,
    generation: i64,
}

impl RepositoryStore {
    pub async fn queue_github_workflow_job_read(
        &self,
        repo_id: &str,
        github_repository_id: u64,
        github_run_id: u64,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        self.db.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_github_workflow_job_reads (repo_id, github_repository_id, github_run_id, attempts, generation, next_attempt_at_unix)
             VALUES ($1, $2, $3, 0, 0, $4)
             ON CONFLICT (repo_id, github_repository_id, github_run_id)
             DO UPDATE SET generation = scope_github_workflow_job_reads.generation + 1",
            [repo_id.into(), u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
             u64_to_i64(github_run_id, "GitHub workflow run id")?.into(),
             u64_to_i64(now_unix, "GitHub jobs read time")?.into()],
        )).await.map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn claim_due_github_workflow_job_reads(
        &self,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubWorkflowJobReadJob>, PostgresError> {
        JobReadRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_github_workflow_job_reads read
                SET attempts = read.attempts + 1, next_attempt_at_unix = $2
               FROM (SELECT repo_id, github_repository_id, github_run_id
                       FROM scope_github_workflow_job_reads
                      WHERE next_attempt_at_unix <= $1
                      ORDER BY next_attempt_at_unix
                      LIMIT $3 FOR UPDATE SKIP LOCKED) due
              WHERE read.repo_id = due.repo_id
                AND read.github_repository_id = due.github_repository_id
                AND read.github_run_id = due.github_run_id
          RETURNING read.read_id, read.repo_id, read.github_repository_id, read.github_run_id, read.attempts, read.generation",
            [
                u64_to_i64(now_unix, "GitHub jobs read time")?.into(),
                u64_to_i64(lease_until_unix, "GitHub jobs read lease")?.into(),
                u64_to_i64(limit, "GitHub jobs read batch size")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| {
            Ok(GitHubWorkflowJobReadJob {
                read_id: i64_to_u64(row.read_id, "GitHub jobs read id")?,
                repo_id: row.repo_id,
                github_repository_id: i64_to_u64(row.github_repository_id, "GitHub repository id")?,
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
        let key: Vec<Value> = vec![
            job.repo_id.clone().into(),
            u64_to_i64(job.github_repository_id, "GitHub repository id")?.into(),
            u64_to_i64(job.github_run_id, "GitHub workflow run id")?.into(),
        ];
        let mut values = key;
        values.push(u32_to_i32(job.attempts, "GitHub jobs read attempts")?.into());
        values.push(u64_to_i64(job.generation, "GitHub jobs read generation")?.into());
        values.push(u64_to_i64(job.read_id, "GitHub jobs read id")?.into());
        if retry_at_unix.is_none() {
            self.db
                .execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "DELETE FROM scope_github_workflow_job_reads
                  WHERE repo_id = $1 AND github_repository_id = $2 AND github_run_id = $3
                    AND attempts = $4 AND generation = $5 AND read_id = $6",
                    values.clone(),
                ))
                .await
                .map_err(PostgresError::internal)?;
        }
        values
            .push(u64_to_i64(retry_at_unix.unwrap_or(now_unix), "GitHub jobs retry time")?.into());
        values.push(u64_to_i64(now_unix, "GitHub jobs read time")?.into());
        let statement = Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_github_workflow_job_reads
                SET next_attempt_at_unix = CASE WHEN generation = $5 THEN $7 ELSE $8 END
              WHERE repo_id = $1 AND github_repository_id = $2 AND github_run_id = $3
                AND attempts = $4 AND read_id = $6",
            values,
        );
        self.db
            .execute_raw(statement)
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
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
        .await
        .map_err(PostgresError::internal)?
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
        .await
        .map_err(PostgresError::internal)?
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
        .await
        .map_err(PostgresError::internal)?
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
            .await
            .map_err(PostgresError::internal)?;
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
    let steps = job
        .steps
        .iter()
        .map(|step| StoredStep {
            number: step.number,
            name: step.name.clone(),
            status: step.status,
            conclusion: step.conclusion,
            started_at_unix: step.started_at_unix,
            completed_at_unix: step.completed_at_unix,
        })
        .collect::<Vec<_>>();
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
            serde_json::to_value(steps)
                .map_err(PostgresError::internal)?
                .into(),
            i16::from(progress.stage).into(),
            u32_to_i32(progress.steps_completed, "GitHub job steps")?.into(),
            u32_to_i32(progress.steps_started, "GitHub job steps")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

impl JobRow {
    fn into_domain(self) -> Result<GitHubWorkflowJob, PostgresError> {
        let steps: Vec<StoredStep> =
            serde_json::from_value(self.steps).map_err(PostgresError::internal)?;
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
            steps: steps
                .into_iter()
                .map(|step| GitHubWorkflowStep {
                    number: step.number,
                    name: step.name,
                    status: step.status,
                    conclusion: step.conclusion,
                    started_at_unix: step.started_at_unix,
                    completed_at_unix: step.completed_at_unix,
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests;
