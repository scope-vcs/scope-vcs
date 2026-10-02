//! Workflow runs GitHub Actions reported for connected repositories. Each
//! read from GitHub replaces the stored run unless what is stored has come
//! further, so a slow read cannot move a run backwards. A run a delivery
//! named but GitHub could not be asked about yet waits as a pending read.

use super::{
    RepositoryStore,
    entities::{decode_enum, encode_enum},
    integer_columns::{
        i32_to_u32, i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, u32_to_i32, u64_to_i64,
    },
};
use crate::error::PostgresError;
use scope_domain::{github_workflow_runs::GitHubWorkflowRun, requests::GitHubBranch};
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

/// A workflow run a delivery named that Scope still has to read from GitHub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRunReadJob {
    pub repo_id: String,
    pub github_repository_id: u64,
    pub github_run_id: u64,
    /// Attempts so far, this one included.
    pub attempts: u32,
}

#[derive(FromQueryResult)]
struct ReadJobRow {
    repo_id: String,
    github_repository_id: i64,
    github_run_id: i64,
    attempts: i32,
}

/// A listed run, and the request whose branch it ran on while that request
/// exists in the repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRunRead {
    pub run: GitHubWorkflowRun,
    pub request_id: Option<String>,
}

/// Which listed runs a page holds.
#[derive(Clone, Copy, Debug)]
pub struct GitHubWorkflowRunPageQuery<'a> {
    pub repo_id: &'a str,
    pub github_repository_id: u64,
    pub workflow_name: Option<&'a str>,
    /// The page starts after this run.
    pub after: Option<GitHubWorkflowRunCursor>,
    pub limit: u64,
}

/// Where a run is in the list: runs list newest first by
/// [`GitHubWorkflowRun::listed_at_unix`], then by id.
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

impl RepositoryStore {
    /// Stores what the GitHub repository reports for a run of its workflows.
    pub async fn save_github_workflow_run(
        &self,
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
        // A run id belongs to one GitHub repository, which may since have been
        // connected to another Scope repository.
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_workflow_runs (github_run_id, repo_id,
                    github_repository_id, workflow_name, head_branch, head_oid, event, status,
                    conclusion, html_url, check_suite_id, run_started_at_unix,
                    github_updated_at_unix, run_attempt, stage)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                 ON CONFLICT (github_run_id) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id,
                    github_repository_id = EXCLUDED.github_repository_id,
                    workflow_name = EXCLUDED.workflow_name,
                    head_branch = EXCLUDED.head_branch, head_oid = EXCLUDED.head_oid,
                    event = EXCLUDED.event, status = EXCLUDED.status,
                    conclusion = EXCLUDED.conclusion, html_url = EXCLUDED.html_url,
                    check_suite_id = EXCLUDED.check_suite_id,
                    run_started_at_unix = EXCLUDED.run_started_at_unix,
                    github_updated_at_unix = EXCLUDED.github_updated_at_unix,
                    run_attempt = EXCLUDED.run_attempt, stage = EXCLUDED.stage
                  WHERE (scope_github_workflow_runs.run_attempt, scope_github_workflow_runs.stage,
                         scope_github_workflow_runs.github_updated_at_unix)
                        <= (EXCLUDED.run_attempt, EXCLUDED.stage, EXCLUDED.github_updated_at_unix)",
                values,
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    /// Keeps a run a delivery named for a later read, unless one is waiting.
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

    /// Takes pending reads that are due for this process, putting their next
    /// try at `retry_at_unix` in case it never reports back.
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

    /// Ends a pending read: answered, or given up when `retry_at_unix` is
    /// `None`. Otherwise it is tried again then.
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

    /// A page of the workflow runs the GitHub repository reported for the
    /// repository, newest first, of one workflow when `workflow_name` names it.
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
                   LEFT JOIN scope_requests request
                     ON request.repo_id = run.repo_id
                    AND run.head_branch = 'scope/requests/' || request.id
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
        .map(|row| {
            let run = row.run.into_domain()?;
            // The join finds the request; the domain decides the run is on its branch.
            let request_id = row
                .request_id
                .filter(|request_id| run.request_id().as_deref() == Some(request_id));
            Ok(GitHubWorkflowRunRead { run, request_id })
        })
        .collect()
    }

    /// The names of the workflows whose runs the GitHub repository reported
    /// for the repository, in name order.
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

/// The runs on one of Scope's branches for one commit.
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
