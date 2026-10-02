//! Workflow runs GitHub Actions reported for connected repositories. Each
//! read from GitHub replaces the stored run unless what is stored is newer,
//! so a slow read cannot move a run backwards.

use super::{
    RepositoryStore,
    entities::{decode_enum, encode_enum},
    integer_columns::{i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::{github_workflow_runs::GitHubWorkflowRun, requests::GitHubBranch};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, Value};

const SELECT_RUN: &str = "run.github_run_id, run.workflow_name, run.head_branch, run.head_oid,
    run.event, run.status, run.conclusion, run.html_url, run.check_suite_id,
    run.run_started_at_unix, run.github_created_at_unix, run.github_updated_at_unix";

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
    github_created_at_unix: i64,
    github_updated_at_unix: i64,
}

/// A listed run, and the request whose branch it ran on while that request
/// exists in the repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRunRead {
    pub run: GitHubWorkflowRun,
    pub request_id: Option<String>,
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
            u64_to_i64(run.updated_at_unix, "GitHub run update")?.into(),
            u64_to_i64(run.created_at_unix, "GitHub run creation")?.into(),
        ];
        // A run id belongs to one GitHub repository, which may since have been
        // connected to another Scope repository.
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_workflow_runs (github_run_id, repo_id,
                    github_repository_id, workflow_name, head_branch, head_oid, event, status,
                    conclusion, html_url, check_suite_id, run_started_at_unix,
                    github_updated_at_unix, github_created_at_unix)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                 ON CONFLICT (github_run_id) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id,
                    github_repository_id = EXCLUDED.github_repository_id,
                    workflow_name = EXCLUDED.workflow_name,
                    head_branch = EXCLUDED.head_branch, head_oid = EXCLUDED.head_oid,
                    event = EXCLUDED.event, status = EXCLUDED.status,
                    conclusion = EXCLUDED.conclusion, html_url = EXCLUDED.html_url,
                    check_suite_id = EXCLUDED.check_suite_id,
                    run_started_at_unix = EXCLUDED.run_started_at_unix,
                    github_created_at_unix = EXCLUDED.github_created_at_unix,
                    github_updated_at_unix = EXCLUDED.github_updated_at_unix
                  WHERE scope_github_workflow_runs.github_updated_at_unix
                        <= EXCLUDED.github_updated_at_unix",
                values,
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    /// The most recent workflow runs the GitHub repository reported for the
    /// repository, newest first.
    pub async fn recent_github_workflow_runs(
        &self,
        repo_id: &str,
        github_repository_id: u64,
        limit: u64,
    ) -> Result<Vec<GitHubWorkflowRunRead>, PostgresError> {
        ListedRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {SELECT_RUN}, request.id AS request_id
                   FROM scope_github_workflow_runs run
                   LEFT JOIN scope_requests request
                     ON request.repo_id = run.repo_id
                    AND run.head_branch = 'scope/requests/' || request.id
                  WHERE run.repo_id = $1 AND run.github_repository_id = $3
                  ORDER BY coalesce(run.run_started_at_unix, run.github_updated_at_unix) DESC,
                           run.github_run_id DESC
                  LIMIT $2"
            ),
            [
                repo_id.into(),
                u64_to_i64(limit, "GitHub workflow run page size")?.into(),
                u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
            ],
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
            created_at_unix: i64_to_u64(self.github_created_at_unix, "GitHub run creation")?,
            updated_at_unix: i64_to_u64(self.github_updated_at_unix, "GitHub run update")?,
        })
    }
}

#[cfg(test)]
mod tests;
