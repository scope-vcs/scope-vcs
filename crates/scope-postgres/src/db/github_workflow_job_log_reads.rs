use super::{
    RepositoryStore,
    github_read_queue::GitHubReadQueue,
    integer_columns::{i32_to_u32, i64_to_u64, u32_to_i32, u64_to_i64},
};
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};

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

const LOG_READS: GitHubReadQueue = GitHubReadQueue {
    table: "scope_github_workflow_job_log_reads",
    key: &["github_job_id"],
    joined: ", scope_github_workflow_jobs job",
    joined_on: " AND job.github_job_id = read.github_job_id",
    returning: "job.repo_id, job.github_repository_id, job.github_run_id, read.github_job_id, read.attempts",
    label: "GitHub job log read",
};

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
            .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_github_job_log_reads"))]
    pub async fn claim_due_github_job_log_reads(
        &self,
        now_unix: u64,
        retry_at_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubJobLogReadJob>, PostgresError> {
        LOG_READS
            .claim_due::<LogReadRow>(self.db.as_ref(), now_unix, retry_at_unix, limit)
            .await?
            .into_iter()
            .map(|row| {
                Ok(GitHubJobLogReadJob {
                    repo_id: row.repo_id,
                    github_repository_id: i64_to_u64(
                        row.github_repository_id,
                        "GitHub repository id",
                    )?,
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
        LOG_READS
            .finish(
                self.db.as_ref(),
                &["attempts"],
                vec![
                    u64_to_i64(read.github_job_id, "GitHub job id")?.into(),
                    u32_to_i32(read.attempts, "GitHub job log read attempts")?.into(),
                ],
                retry_at_unix,
            )
            .await
    }
}

#[cfg(test)]
mod tests;
