use super::{
    RepositoryStore,
    entities::{decode_enum, encode_enum},
    integer_columns::{
        i32_to_u32, i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, u32_to_i32, u64_to_i64,
    },
};
use crate::error::PostgresError;
use scope_domain::{
    github_workflow_jobs::GitHubJobsRead, github_workflow_runs::GitHubWorkflowRun,
    requests::GitHubBranch,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, Value};

const SELECT_RUN: &str = "run.github_run_id, run.workflow_name, run.head_branch, run.head_oid,
    run.event, run.status, run.conclusion, run.html_url, run.check_suite_id,
    run.run_started_at_unix, run.run_attempt, run.github_updated_at_unix";

#[derive(FromQueryResult)]
struct WorkflowRunRow {
    github_run_id: i64,
    workflow_name: String,
    head_branch: Option<String>,
    head_oid: String,
    event: String,
    status: String,
    conclusion: Option<String>,
    html_url: String,
    check_suite_id: Option<i64>,
    run_started_at_unix: Option<i64>,
    run_attempt: i32,
    github_updated_at_unix: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRunReadJob {
    pub repo_id: String,
    pub github_repository_id: u64,
    pub github_run_id: u64,
    pub attempts: u32,
}

#[derive(FromQueryResult)]
struct ReadJobRow {
    repo_id: String,
    github_repository_id: i64,
    github_run_id: i64,
    attempts: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRunRead {
    pub run: GitHubWorkflowRun,
    pub request_id: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct GitHubWorkflowRunPageQuery<'a> {
    pub repo_id: &'a str,
    pub github_repository_id: u64,
    pub workflow_name: Option<&'a str>,
    pub after: Option<GitHubWorkflowRunCursor>,
    pub limit: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRunCursor {
    pub listed_at_unix: u64,
    pub github_run_id: u64,
}

#[derive(FromQueryResult)]
struct ListedRow {
    #[sea_orm(nested)]
    run: WorkflowRunRow,
    request_id: Option<String>,
}

#[derive(FromQueryResult)]
struct DetailRow {
    #[sea_orm(nested)]
    listed: ListedRow,
    jobs_read_attempt: Option<i32>,
    jobs_read_at_unix: Option<i64>,
    jobs_read_queued: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRunDetailRead {
    pub read: GitHubWorkflowRunRead,
    pub jobs_read: Option<GitHubJobsRead>,
    pub jobs_read_queued: bool,
}

const REQUEST_JOIN: &str = "LEFT JOIN scope_requests request
      ON request.repo_id = run.repo_id
     AND request.id = run.request_id";

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "save_github_workflow_run"))]
    pub async fn save_github_workflow_run(
        &self,
        repo_id: &str,
        github_repository_id: u64,
        run: &GitHubWorkflowRun,
    ) -> Result<(), PostgresError> {
        save_workflow_run(self.db.as_ref(), repo_id, github_repository_id, run).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "queue_github_workflow_run_read"))]
    pub async fn queue_github_workflow_run_read(
        &self,
        repo_id: &str,
        github_repository_id: u64,
        github_run_id: u64,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_workflow_run_reads (repo_id, github_repository_id,
                    github_run_id, attempts, next_attempt_at_unix)
                 VALUES ($1, $2, $3, 0, $4)
                 ON CONFLICT DO NOTHING",
                [
                    repo_id.into(),
                    u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
                    u64_to_i64(github_run_id, "GitHub workflow run id")?.into(),
                    u64_to_i64(now_unix, "GitHub workflow run read time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_github_workflow_run_reads"))]
    pub async fn claim_due_github_workflow_run_reads(
        &self,
        now_unix: u64,
        retry_at_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubWorkflowRunReadJob>, PostgresError> {
        ReadJobRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_github_workflow_run_reads read
                SET attempts = read.attempts + 1, next_attempt_at_unix = $2
               FROM (SELECT repo_id, github_repository_id, github_run_id
                       FROM scope_github_workflow_run_reads
                      WHERE next_attempt_at_unix <= $1
                      ORDER BY next_attempt_at_unix
                      LIMIT $3
                      FOR UPDATE SKIP LOCKED) due
              WHERE read.repo_id = due.repo_id
                AND read.github_repository_id = due.github_repository_id
                AND read.github_run_id = due.github_run_id
          RETURNING read.repo_id, read.github_repository_id, read.github_run_id, read.attempts",
            [
                u64_to_i64(now_unix, "GitHub workflow run read time")?.into(),
                u64_to_i64(retry_at_unix, "GitHub workflow run read time")?.into(),
                u64_to_i64(limit, "GitHub workflow run read batch size")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| {
            Ok(GitHubWorkflowRunReadJob {
                repo_id: row.repo_id,
                github_repository_id: i64_to_u64(row.github_repository_id, "GitHub repository id")?,
                github_run_id: i64_to_u64(row.github_run_id, "GitHub workflow run id")?,
                attempts: i32_to_u32(row.attempts, "GitHub workflow run read attempts")?,
            })
        })
        .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "finish_github_workflow_run_read"))]
    pub async fn finish_github_workflow_run_read(
        &self,
        job: &GitHubWorkflowRunReadJob,
        retry_at_unix: Option<u64>,
    ) -> Result<(), PostgresError> {
        let key: Vec<Value> = vec![
            job.repo_id.clone().into(),
            u64_to_i64(job.github_repository_id, "GitHub repository id")?.into(),
            u64_to_i64(job.github_run_id, "GitHub workflow run id")?.into(),
        ];
        let statement = match retry_at_unix {
            None => Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "DELETE FROM scope_github_workflow_run_reads
                  WHERE repo_id = $1 AND github_repository_id = $2 AND github_run_id = $3",
                key,
            ),
            Some(retry_at) => {
                let mut values = key;
                values.push(u64_to_i64(retry_at, "GitHub workflow run read time")?.into());
                Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "UPDATE scope_github_workflow_run_reads SET next_attempt_at_unix = $4
                      WHERE repo_id = $1 AND github_repository_id = $2 AND github_run_id = $3",
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

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_workflow_run_page"))]
    pub async fn github_workflow_run_page(
        &self,
        query: GitHubWorkflowRunPageQuery<'_>,
    ) -> Result<Vec<GitHubWorkflowRunRead>, PostgresError> {
        let mut values: Vec<Value> = vec![
            query.repo_id.into(),
            u64_to_i64(query.github_repository_id, "GitHub repository id")?.into(),
            u64_to_i64(query.limit, "GitHub workflow run page size")?.into(),
        ];
        let mut filters = String::new();
        if let Some(workflow_name) = query.workflow_name {
            values.push(workflow_name.into());
            filters.push_str(&format!(" AND run.workflow_name = ${}", values.len()));
        }
        if let Some(after) = query.after {
            values.push(u64_to_i64(after.listed_at_unix, "GitHub workflow run cursor")?.into());
            values.push(u64_to_i64(after.github_run_id, "GitHub workflow run cursor")?.into());
            filters.push_str(&format!(
                " AND (coalesce(run.run_started_at_unix, run.github_updated_at_unix),
                       run.github_run_id) < (${}, ${})",
                values.len() - 1,
                values.len()
            ));
        }
        ListedRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {SELECT_RUN}, request.id AS request_id
                   FROM scope_github_workflow_runs run
                   {REQUEST_JOIN}
                  WHERE run.repo_id = $1 AND run.github_repository_id = $2{filters}
                  ORDER BY coalesce(run.run_started_at_unix, run.github_updated_at_unix) DESC,
                           run.github_run_id DESC
                  LIMIT $3"
            ),
            values,
        ))
        .all(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(ListedRow::into_read)
        .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_workflow_run"))]
    pub async fn github_workflow_run(
        &self,
        repo_id: &str,
        github_repository_id: u64,
        github_run_id: u64,
    ) -> Result<Option<GitHubWorkflowRunDetailRead>, PostgresError> {
        DetailRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {SELECT_RUN}, request.id AS request_id,
                        run.jobs_read_attempt, run.jobs_read_at_unix,
                        EXISTS (SELECT 1 FROM scope_github_workflow_job_reads queued
                                 WHERE queued.repo_id = run.repo_id
                                   AND queued.github_repository_id = run.github_repository_id
                                   AND queued.github_run_id = run.github_run_id) AS jobs_read_queued
                   FROM scope_github_workflow_runs run
                   {REQUEST_JOIN}
                  WHERE run.repo_id = $1 AND run.github_repository_id = $2
                    AND run.github_run_id = $3"
            ),
            [
                repo_id.into(),
                u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
                u64_to_i64(github_run_id, "GitHub workflow run id")?.into(),
            ],
        ))
        .one(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .map(|row| {
            let jobs_read = match (row.jobs_read_attempt, row.jobs_read_at_unix) {
                (Some(attempt), Some(read_at)) => Some(GitHubJobsRead {
                    run_attempt: i32_to_u32(attempt, "GitHub jobs read attempt")?,
                    read_at_unix: i64_to_u64(read_at, "GitHub jobs read time")?,
                }),
                _ => None,
            };
            Ok(GitHubWorkflowRunDetailRead {
                read: row.listed.into_read()?,
                jobs_read,
                jobs_read_queued: row.jobs_read_queued,
            })
        })
        .transpose()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "replace_github_jobs_read"))]
    pub async fn replace_github_jobs_read(
        &self,
        repo_id: &str,
        github_run_id: u64,
        expected: Option<GitHubJobsRead>,
        next: Option<GitHubJobsRead>,
    ) -> Result<bool, PostgresError> {
        let attempt = |read: Option<GitHubJobsRead>| {
            read.map(|read| u32_to_i32(read.run_attempt, "GitHub jobs read attempt"))
                .transpose()
        };
        let read_at = |read: Option<GitHubJobsRead>| {
            read.map(|read| u64_to_i64(read.read_at_unix, "GitHub jobs read time"))
                .transpose()
        };
        let result = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_github_workflow_runs
                    SET jobs_read_attempt = $3, jobs_read_at_unix = $4
                  WHERE repo_id = $1 AND github_run_id = $2
                    AND jobs_read_attempt IS NOT DISTINCT FROM $5
                    AND jobs_read_at_unix IS NOT DISTINCT FROM $6",
                [
                    repo_id.into(),
                    u64_to_i64(github_run_id, "GitHub workflow run id")?.into(),
                    attempt(next)?.into(),
                    read_at(next)?.into(),
                    attempt(expected)?.into(),
                    read_at(expected)?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected() == 1)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_workflow_runs_for_check_suites"))]
    pub async fn github_workflow_runs_for_check_suites(
        &self,
        repo_id: &str,
        github_repository_id: u64,
        check_suite_ids: &[u64],
    ) -> Result<Vec<(u64, u64)>, PostgresError> {
        github_workflow_runs_for_check_suites(
            self.db.as_ref(),
            repo_id,
            github_repository_id,
            check_suite_ids,
        )
        .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_workflow_names"))]
    pub async fn github_workflow_names(
        &self,
        repo_id: &str,
        github_repository_id: u64,
    ) -> Result<Vec<String>, PostgresError> {
        self.db
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT DISTINCT workflow_name FROM scope_github_workflow_runs
                  WHERE repo_id = $1 AND github_repository_id = $2
                  ORDER BY workflow_name",
                [
                    repo_id.into(),
                    u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| {
                row.try_get("", "workflow_name")
                    .map_err(PostgresError::internal)
            })
            .collect()
    }
}

pub(super) async fn save_workflow_run<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    github_repository_id: u64,
    run: &GitHubWorkflowRun,
) -> Result<(), PostgresError> {
    let progress = run.progress();
    let values: Vec<Value> = vec![
        u64_to_i64(run.github_run_id, "GitHub workflow run id")?.into(),
        repo_id.into(),
        u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
        run.workflow_name.clone().into(),
        run.head_branch.clone().into(),
        run.request_id().into(),
        run.head_oid.clone().into(),
        run.event.clone().into(),
        encode_enum(run.status)?.into(),
        run.conclusion.map(encode_enum).transpose()?.into(),
        run.html_url.clone().into(),
        optional_u64_to_i64(run.check_suite_id, "GitHub check suite id")?.into(),
        optional_u64_to_i64(run.run_started_at_unix, "GitHub run start")?.into(),
        u64_to_i64(progress.updated_at_unix, "GitHub run update")?.into(),
        u32_to_i32(progress.run_attempt, "GitHub run attempt")?.into(),
        i16::from(progress.stage).into(),
    ];
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_github_workflow_runs (github_run_id, repo_id,
                github_repository_id, workflow_name, head_branch, request_id, head_oid, event, status,
                conclusion, html_url, check_suite_id, run_started_at_unix,
                github_updated_at_unix, run_attempt, stage)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
             ON CONFLICT (github_run_id) DO UPDATE SET
                repo_id = EXCLUDED.repo_id,
                github_repository_id = EXCLUDED.github_repository_id,
                workflow_name = EXCLUDED.workflow_name,
                head_branch = EXCLUDED.head_branch, request_id = EXCLUDED.request_id,
                head_oid = EXCLUDED.head_oid,
                event = EXCLUDED.event, status = EXCLUDED.status,
                conclusion = EXCLUDED.conclusion, html_url = EXCLUDED.html_url,
                check_suite_id = EXCLUDED.check_suite_id,
                run_started_at_unix = EXCLUDED.run_started_at_unix,
                github_updated_at_unix = EXCLUDED.github_updated_at_unix,
                run_attempt = EXCLUDED.run_attempt, stage = EXCLUDED.stage
              WHERE (scope_github_workflow_runs.run_attempt, scope_github_workflow_runs.stage,
                     scope_github_workflow_runs.github_updated_at_unix)
                    <= (EXCLUDED.run_attempt, EXCLUDED.stage, EXCLUDED.github_updated_at_unix)
                AND (scope_github_workflow_runs.repo_id = EXCLUDED.repo_id
                     OR EXISTS (
                         SELECT 1 FROM scope_github_connections connection
                          WHERE connection.repo_id = EXCLUDED.repo_id
                            AND connection.github_repository_id = EXCLUDED.github_repository_id
                            AND connection.status = 'Connected'))",
        values,
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn branch_workflow_runs<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    github_repository_id: u64,
    branch: &GitHubBranch,
    head_oid: &str,
) -> Result<Vec<GitHubWorkflowRun>, PostgresError> {
    WorkflowRunRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT {SELECT_RUN}
               FROM scope_github_workflow_runs run
              WHERE run.repo_id = $1 AND run.github_repository_id = $4
                AND run.head_branch = $2 AND run.head_oid = $3
              ORDER BY run.github_run_id"
        ),
        [
            repo_id.into(),
            branch.name().into(),
            head_oid.into(),
            u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
        ],
    ))
    .all(conn)
    .await
    .map_err(PostgresError::internal)?
    .into_iter()
    .map(WorkflowRunRow::into_domain)
    .collect()
}

impl ListedRow {
    fn into_read(self) -> Result<GitHubWorkflowRunRead, PostgresError> {
        let run = self.run.into_domain()?;
        let request_id = self
            .request_id
            .filter(|request_id| run.request_id().as_deref() == Some(request_id));
        Ok(GitHubWorkflowRunRead { run, request_id })
    }
}

impl WorkflowRunRow {
    fn into_domain(self) -> Result<GitHubWorkflowRun, PostgresError> {
        Ok(GitHubWorkflowRun {
            github_run_id: i64_to_u64(self.github_run_id, "GitHub workflow run id")?,
            workflow_name: self.workflow_name,
            head_branch: self.head_branch,
            head_oid: self.head_oid,
            event: self.event,
            status: decode_enum(self.status)?,
            conclusion: self.conclusion.map(decode_enum).transpose()?,
            html_url: self.html_url,
            check_suite_id: optional_i64_to_u64(self.check_suite_id, "GitHub check suite id")?,
            run_started_at_unix: optional_i64_to_u64(self.run_started_at_unix, "GitHub run start")?,
            run_attempt: i32_to_u32(self.run_attempt, "GitHub run attempt")?,
            updated_at_unix: i64_to_u64(self.github_updated_at_unix, "GitHub run update")?,
        })
    }
}

#[cfg(test)]
mod tests;

pub(super) async fn github_workflow_runs_for_check_suites<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    github_repository_id: u64,
    check_suite_ids: &[u64],
) -> Result<Vec<(u64, u64)>, PostgresError> {
    if check_suite_ids.is_empty() {
        return Ok(Vec::new());
    }
    let suites = check_suite_ids
        .iter()
        .map(|id| u64_to_i64(*id, "GitHub check suite id"))
        .collect::<Result<Vec<_>, _>>()?;
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT check_suite_id, github_run_id FROM scope_github_workflow_runs
                  WHERE repo_id = $1 AND github_repository_id = $2
                    AND check_suite_id = ANY($3)",
        [
            repo_id.into(),
            u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
            suites.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?
    .into_iter()
    .map(|row| {
        let suite: i64 = row
            .try_get("", "check_suite_id")
            .map_err(PostgresError::internal)?;
        let run: i64 = row
            .try_get("", "github_run_id")
            .map_err(PostgresError::internal)?;
        Ok((
            i64_to_u64(suite, "GitHub check suite id")?,
            i64_to_u64(run, "GitHub workflow run id")?,
        ))
    })
    .collect()
}
