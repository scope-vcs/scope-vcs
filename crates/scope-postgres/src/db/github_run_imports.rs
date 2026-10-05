use super::{
    RepositoryStore, acquire_aggregate_lock,
    github_connections::repository_github_connection,
    github_workflow_runs::save_workflow_run,
    integer_columns::{i32_to_u32, i64_to_u64, optional_i64_to_u64, u32_to_i32, u64_to_i64},
    locks::acquire_shared_repository_lock,
    repository_access::repository_access,
};
use crate::error::PostgresError;
use scope_domain::{
    github_run_import::{
        GITHUB_RUN_IMPORT_DEFAULT_COUNT, GitHubRunImport, GitHubRunImportState,
        github_run_import_error, set_github_run_import_count, start_github_run_import,
    },
    github_workflow_runs::GitHubWorkflowRun,
    repository::RepositoryIncarnation,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, TransactionTrait};

const SELECT_IMPORT: &str = "repo_id, github_repository_id, run_count, state, attempts, \
    imported_count, last_error, queued_at_unix, finished_at_unix";

#[derive(FromQueryResult)]
struct ImportRow {
    repo_id: String,
    github_repository_id: i64,
    run_count: i32,
    state: String,
    attempts: i32,
    imported_count: i32,
    last_error: Option<String>,
    queued_at_unix: i64,
    finished_at_unix: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubRunImportOutcome {
    Succeeded {
        imported: u32,
    },
    Failed {
        error: String,
        retry_at_unix: Option<u64>,
    },
}

impl RepositoryStore {
    pub async fn github_run_import_count(&self, repo_id: &str) -> Result<u32, PostgresError> {
        run_import_count(self.db.as_ref(), repo_id).await
    }

    pub async fn set_github_run_import_count(
        &self,
        repo_id: &str,
        user_id: &str,
        count: u32,
    ) -> Result<(u32, RepositoryIncarnation), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_shared_repository_lock(&tx, repo_id).await?;
        acquire_aggregate_lock(&tx, "github-run-import", repo_id).await?;
        let context = repository_access(&tx, repo_id, Some(user_id))
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        let count = set_github_run_import_count(context.access, count)?;
        save_run_import_count(&tx, repo_id, count).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok((count, context.incarnation()))
    }

    pub async fn start_github_run_import(
        &self,
        repo_id: &str,
        user_id: &str,
        now_unix: u64,
    ) -> Result<(GitHubRunImport, RepositoryIncarnation), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_shared_repository_lock(&tx, repo_id).await?;
        acquire_aggregate_lock(&tx, "github-connection", repo_id).await?;
        acquire_aggregate_lock(&tx, "github-run-import", repo_id).await?;
        let context = repository_access(&tx, repo_id, Some(user_id))
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        let connection = repository_github_connection(&tx, repo_id).await?;
        let current = load_import(&tx, repo_id).await?;
        let count = run_import_count(&tx, repo_id).await?;
        let import = start_github_run_import(
            context.access,
            connection.as_ref(),
            current.as_ref(),
            count,
            now_unix,
        )?;
        queue_github_run_import(&tx, &import).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok((import, context.incarnation()))
    }

    pub async fn github_run_import(
        &self,
        repo_id: &str,
    ) -> Result<Option<GitHubRunImport>, PostgresError> {
        load_import(self.db.as_ref(), repo_id).await
    }

    pub async fn claim_due_github_run_imports(
        &self,
        claim_token: &str,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubRunImport>, PostgresError> {
        if lease_until_unix <= now_unix {
            return Err(PostgresError::invalid_input(
                "GitHub run import lease must end in the future",
            ));
        }
        ImportRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "UPDATE scope_github_run_imports import
                    SET state = 'running', lease_until_unix = $2, claim_token = $3,
                        next_attempt_at_unix = $2, attempts = import.attempts + 1
                   FROM (SELECT repo_id
                           FROM scope_github_run_imports
                          WHERE (state = 'queued' AND next_attempt_at_unix <= $1)
                             OR (state = 'running' AND lease_until_unix <= $1)
                          ORDER BY next_attempt_at_unix, repo_id
                          LIMIT $4
                          FOR UPDATE SKIP LOCKED) due
                  WHERE import.repo_id = due.repo_id
              RETURNING {}",
                prefixed("import.")
            ),
            [
                u64_to_i64(now_unix, "GitHub run import claim time")?.into(),
                u64_to_i64(lease_until_unix, "GitHub run import lease")?.into(),
                claim_token.into(),
                u64_to_i64(limit, "GitHub run import batch size")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(ImportRow::into_domain)
        .collect()
    }

    pub async fn store_github_run_import_page(
        &self,
        repo_id: &str,
        claim_token: &str,
        runs: &[GitHubWorkflowRun],
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(row) = tx
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT import.github_repository_id
                   FROM scope_github_run_imports import
                   JOIN scope_github_connections connection
                     ON connection.repo_id = import.repo_id
                    AND connection.github_repository_id = import.github_repository_id
                    AND connection.status = 'Connected'
                  WHERE import.repo_id = $1 AND import.claim_token = $2
                    AND import.state = 'running'
                    AND import.lease_until_unix > extract(epoch FROM now())::bigint
                    FOR UPDATE OF import FOR SHARE OF connection",
                [repo_id.into(), claim_token.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(false);
        };
        let github_repository_id = i64_to_u64(
            row.try_get("", "github_repository_id")
                .map_err(PostgresError::internal)?,
            "GitHub repository id",
        )?;
        for run in runs {
            save_workflow_run(&tx, repo_id, github_repository_id, run).await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(true)
    }

    pub async fn finish_github_run_import(
        &self,
        repo_id: &str,
        claim_token: &str,
        outcome: GitHubRunImportOutcome,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let now = u64_to_i64(now_unix, "GitHub run import time")?;
        let (state, imported, error, retry_at, finished) = match outcome {
            GitHubRunImportOutcome::Succeeded { imported } => {
                ("succeeded", imported, None, None, Some(now))
            }
            GitHubRunImportOutcome::Failed {
                error,
                retry_at_unix: Some(retry_at),
            } => (
                "queued",
                0,
                Some(github_run_import_error(&error)),
                Some(u64_to_i64(retry_at, "GitHub run import retry time")?),
                None,
            ),
            GitHubRunImportOutcome::Failed {
                error,
                retry_at_unix: None,
            } => (
                "failed",
                0,
                Some(github_run_import_error(&error)),
                None,
                Some(now),
            ),
        };
        let result = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_github_run_imports
                    SET state = $3, imported_count = least($4, run_count), last_error = $5,
                        next_attempt_at_unix = coalesce($6, next_attempt_at_unix),
                        finished_at_unix = CASE WHEN $7::bigint IS NULL THEN NULL
                                           ELSE greatest($7, queued_at_unix) END,
                        lease_until_unix = NULL, claim_token = NULL
                  WHERE repo_id = $1 AND claim_token = $2 AND state = 'running'",
                [
                    repo_id.into(),
                    claim_token.into(),
                    state.into(),
                    u32_to_i32(imported, "GitHub run import count")?.into(),
                    error.into(),
                    retry_at.into(),
                    finished.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected() == 1)
    }
}

pub(super) async fn run_import_count<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<u32, PostgresError> {
    match conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT run_count FROM scope_github_run_import_counts WHERE repo_id = $1",
            [repo_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
    {
        Some(row) => i32_to_u32(
            row.try_get("", "run_count")
                .map_err(PostgresError::internal)?,
            "GitHub run import count",
        ),
        None => Ok(GITHUB_RUN_IMPORT_DEFAULT_COUNT),
    }
}

pub(super) async fn save_run_import_count<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    count: u32,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_github_run_import_counts (repo_id, run_count) VALUES ($1, $2)
         ON CONFLICT (repo_id) DO UPDATE SET run_count = EXCLUDED.run_count",
        [
            repo_id.into(),
            u32_to_i32(count, "GitHub run import count")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn queue_github_run_import<C: ConnectionTrait>(
    conn: &C,
    import: &GitHubRunImport,
) -> Result<(), PostgresError> {
    if import.state != GitHubRunImportState::Queued {
        return Err(PostgresError::internal_message(
            "only a queued GitHub run import can be queued",
        ));
    }
    let queued_at = u64_to_i64(import.queued_at_unix, "GitHub run import time")?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_github_run_imports (repo_id, github_repository_id, run_count, state,
            attempts, imported_count, last_error, next_attempt_at_unix, lease_until_unix,
            claim_token, queued_at_unix, finished_at_unix)
         VALUES ($1, $2, $3, 'queued', 0, 0, NULL, $4, NULL, NULL, $4, NULL)
         ON CONFLICT (repo_id) DO UPDATE SET
            github_repository_id = EXCLUDED.github_repository_id,
            run_count = EXCLUDED.run_count, state = EXCLUDED.state,
            attempts = EXCLUDED.attempts, imported_count = EXCLUDED.imported_count,
            last_error = EXCLUDED.last_error,
            next_attempt_at_unix = EXCLUDED.next_attempt_at_unix,
            lease_until_unix = EXCLUDED.lease_until_unix,
            claim_token = EXCLUDED.claim_token, queued_at_unix = EXCLUDED.queued_at_unix,
            finished_at_unix = EXCLUDED.finished_at_unix",
        [
            import.repository_id.clone().into(),
            u64_to_i64(import.github_repository_id, "GitHub repository id")?.into(),
            u32_to_i32(import.run_count, "GitHub run import count")?.into(),
            queued_at.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn delete_github_run_import<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM scope_github_run_imports WHERE repo_id = $1",
        [repo_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

async fn load_import<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<GitHubRunImport>, PostgresError> {
    ImportRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!("SELECT {SELECT_IMPORT} FROM scope_github_run_imports WHERE repo_id = $1"),
        [repo_id.into()],
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    .map(ImportRow::into_domain)
    .transpose()
}

fn prefixed(prefix: &str) -> String {
    SELECT_IMPORT
        .split(", ")
        .map(|column| format!("{prefix}{}", column.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

impl ImportRow {
    fn into_domain(self) -> Result<GitHubRunImport, PostgresError> {
        Ok(GitHubRunImport {
            repository_id: self.repo_id,
            github_repository_id: i64_to_u64(self.github_repository_id, "GitHub repository id")?,
            run_count: i32_to_u32(self.run_count, "GitHub run import count")?,
            state: match self.state.as_str() {
                "queued" => GitHubRunImportState::Queued,
                "running" => GitHubRunImportState::Running,
                "succeeded" => GitHubRunImportState::Succeeded,
                "failed" => GitHubRunImportState::Failed,
                other => {
                    return Err(PostgresError::internal_message(format!(
                        "unknown GitHub run import state {other}"
                    )));
                }
            },
            attempts: i32_to_u32(self.attempts, "GitHub run import attempts")?,
            imported_count: i32_to_u32(self.imported_count, "GitHub run import count")?,
            last_error: self.last_error,
            queued_at_unix: i64_to_u64(self.queued_at_unix, "GitHub run import time")?,
            finished_at_unix: optional_i64_to_u64(self.finished_at_unix, "GitHub run import end")?,
        })
    }
}

#[cfg(test)]
mod tests;
