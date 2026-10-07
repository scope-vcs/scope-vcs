use super::{
    RepositoryStore,
    integer_columns::{i32_to_u32, i64_to_u64, u32_to_i32, u64_to_i64},
};
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubJobLogReadJob {
    pub repo_id: String,
    pub github_repository_id: u64,
    pub github_run_id: u64,
    pub github_job_id: u64,
    pub attempts: u32,
}

#[derive(FromQueryResult)]
struct LogReadRow {
    repo_id: String,
    github_repository_id: i64,
    github_run_id: i64,
    github_job_id: i64,
    attempts: i32,
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "queue_github_job_log_read"))]
    pub async fn queue_github_job_log_read(
        &self,
        github_job_id: u64,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_workflow_job_log_reads
                    (github_job_id, attempts, next_attempt_at_unix)
                 SELECT $1, 0, $2
                  WHERE NOT EXISTS (SELECT 1 FROM scope_github_workflow_job_logs
                                     WHERE github_job_id = $1)
                 ON CONFLICT DO NOTHING",
                [
                    u64_to_i64(github_job_id, "GitHub job id")?.into(),
                    u64_to_i64(now_unix, "GitHub job log read time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_github_job_log_reads"))]
    pub async fn claim_due_github_job_log_reads(
        &self,
        now_unix: u64,
        retry_at_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubJobLogReadJob>, PostgresError> {
        LogReadRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_github_workflow_job_log_reads read
                SET attempts = read.attempts + 1, next_attempt_at_unix = $2
               FROM (SELECT github_job_id
                       FROM scope_github_workflow_job_log_reads
                      WHERE next_attempt_at_unix <= $1
                      ORDER BY next_attempt_at_unix
                      LIMIT $3
                      FOR UPDATE SKIP LOCKED) due,
                    scope_github_workflow_jobs job
              WHERE read.github_job_id = due.github_job_id
                AND job.github_job_id = read.github_job_id
          RETURNING job.repo_id, job.github_repository_id, job.github_run_id,
                    read.github_job_id, read.attempts",
            [
                u64_to_i64(now_unix, "GitHub job log read time")?.into(),
                u64_to_i64(retry_at_unix, "GitHub job log read time")?.into(),
                u64_to_i64(limit, "GitHub job log read batch size")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| {
            Ok(GitHubJobLogReadJob {
                repo_id: row.repo_id,
                github_repository_id: i64_to_u64(row.github_repository_id, "GitHub repository id")?,
                github_run_id: i64_to_u64(row.github_run_id, "GitHub workflow run id")?,
                github_job_id: i64_to_u64(row.github_job_id, "GitHub job id")?,
                attempts: i32_to_u32(row.attempts, "GitHub job log read attempts")?,
            })
        })
        .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "finish_github_job_log_read"))]
    pub async fn finish_github_job_log_read(
        &self,
        read: &GitHubJobLogReadJob,
        retry_at_unix: Option<u64>,
    ) -> Result<(), PostgresError> {
        let claim: Vec<Value> = vec![
            u64_to_i64(read.github_job_id, "GitHub job id")?.into(),
            u32_to_i32(read.attempts, "GitHub job log read attempts")?.into(),
        ];
        let statement = match retry_at_unix {
            None => Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "DELETE FROM scope_github_workflow_job_log_reads
                  WHERE github_job_id = $1 AND attempts = $2",
                claim,
            ),
            Some(retry_at) => {
                let mut values = claim;
                values.push(u64_to_i64(retry_at, "GitHub job log read time")?.into());
                Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "UPDATE scope_github_workflow_job_log_reads SET next_attempt_at_unix = $3
                      WHERE github_job_id = $1 AND attempts = $2",
                    values,
                )
            }
        };
        self.db
            .execute_raw(statement)
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
