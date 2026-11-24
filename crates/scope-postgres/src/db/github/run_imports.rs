use super::super::{
    RepositoryStore,
    entities::decode_enum,
    github::connections::repository_github_connection,
    github::import_queue::GitHubImportQueue,
    github::read_queue::GitHubJobOutcome,
    github::workflow_runs::save_workflow_run,
    integer_columns::{i32_to_u32, i64_to_u64, optional_i64_to_u64, u32_to_i32, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::{
    github::run_import::{
        GITHUB_RUN_IMPORT_DEFAULT_COUNT, GitHubRunImport, set_github_run_import_count,
        start_github_run_import,
    },
    github::workflow_runs::GitHubWorkflowRun,
    repository::RepositoryIncarnation,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, TransactionTrait};

const RUN_IMPORTS: GitHubImportQueue = GitHubImportQueue {
    table: "scope_github_run_imports",
    columns: "repo_id, github_repository_id, run_count, state, attempts, imported_count, \
        last_error, queued_at_unix, finished_at_unix",
    label: "GitHub run import",
};

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

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_run_import_count"))]
    pub async fn github_run_import_count(&self, repo_id: &str) -> Result<u32, PostgresError> {
        run_import_count(self.db.as_ref(), repo_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "set_github_run_import_count"))]
    pub async fn set_github_run_import_count(
        &self,
        repo_id: &str,
        user_id: &str,
        count: u32,
    ) -> Result<(u32, RepositoryIncarnation), PostgresError> {
        let (tx, context) = self
            .begin_github_change(repo_id, user_id, &["github-run-import"])
            .await?;
        let count = set_github_run_import_count(context.access.clone(), count)?;
        save_run_import_count(&tx, repo_id, count).await?;
        tx.commit().await?;
        Ok((count, context.incarnation()))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "start_github_run_import"))]
    pub async fn start_github_run_import(
        &self,
        repo_id: &str,
        user_id: &str,
        now_unix: u64,
    ) -> Result<(GitHubRunImport, RepositoryIncarnation), PostgresError> {
        let (tx, context) = self
            .begin_github_change(
                repo_id,
                user_id,
                &["github-connection", "github-run-import"],
            )
            .await?;
        let connection = repository_github_connection(&tx, repo_id).await?;
        let current = load_import(&tx, repo_id).await?;
        let count = run_import_count(&tx, repo_id).await?;
        let import = start_github_run_import(
            context.access.clone(),
            connection.as_ref(),
            current.as_ref(),
            count,
            now_unix,
        )?;
        queue_github_run_import(&tx, &import).await?;
        tx.commit().await?;
        Ok((import, context.incarnation()))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_run_import"))]
    pub async fn github_run_import(
        &self,
        repo_id: &str,
    ) -> Result<Option<GitHubRunImport>, PostgresError> {
        load_import(self.db.as_ref(), repo_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_github_run_imports"))]
    pub async fn claim_due_github_run_imports(
        &self,
        claim_token: &str,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubRunImport>, PostgresError> {
        RUN_IMPORTS
            .claim_due::<ImportRow>(
                self.db.as_ref(),
                claim_token,
                now_unix,
                lease_until_unix,
                limit,
            )
            .await?
            .into_iter()
            .map(ImportRow::into_domain)
            .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "store_github_run_import_page"))]
    pub async fn store_github_run_import_page(
        &self,
        repo_id: &str,
        claim_token: &str,
        runs: &[GitHubWorkflowRun],
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await?;
        let Some(import) = RUN_IMPORTS
            .lock_claimed::<ImportRow>(&tx, repo_id, claim_token)
            .await?
        else {
            return Ok(false);
        };
        let github_repository_id = i64_to_u64(import.github_repository_id, "GitHub repository id")?;
        for run in runs {
            save_workflow_run(&tx, repo_id, github_repository_id, run, now_unix).await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "finish_github_run_import"))]
    pub async fn finish_github_run_import(
        &self,
        repo_id: &str,
        claim_token: &str,
        outcome: GitHubJobOutcome<u32>,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await?;
        if let GitHubJobOutcome::Succeeded(imported) = outcome {
            tx.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_github_run_imports
                    SET imported_count = least($3, run_count)
                  WHERE repo_id = $1 AND claim_token = $2 AND state = 'running'",
                [
                    repo_id.into(),
                    claim_token.into(),
                    u32_to_i32(imported, "GitHub run import count")?.into(),
                ],
            ))
            .await?;
        }
        let finished = RUN_IMPORTS
            .finish(&tx, repo_id, claim_token, &outcome, now_unix)
            .await?;
        tx.commit().await?;
        Ok(finished)
    }
}

pub(in crate::db) async fn run_import_count<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<u32, PostgresError> {
    match conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT run_count FROM scope_github_run_import_counts WHERE repo_id = $1",
            [repo_id.into()],
        ))
        .await?
    {
        Some(row) => i32_to_u32(row.try_get("", "run_count")?, "GitHub run import count"),
        None => Ok(GITHUB_RUN_IMPORT_DEFAULT_COUNT),
    }
}

pub(in crate::db) async fn save_run_import_count<C: ConnectionTrait>(
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
    .await?;
    Ok(())
}

pub(in crate::db) async fn queue_github_run_import<C: ConnectionTrait>(
    conn: &C,
    import: &GitHubRunImport,
) -> Result<(), PostgresError> {
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
    .await?;
    Ok(())
}

pub(in crate::db) async fn delete_github_run_import<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError> {
    RUN_IMPORTS.delete(conn, repo_id).await
}

async fn load_import<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<GitHubRunImport>, PostgresError> {
    RUN_IMPORTS
        .load::<ImportRow>(conn, repo_id)
        .await?
        .map(ImportRow::into_domain)
        .transpose()
}

impl ImportRow {
    fn into_domain(self) -> Result<GitHubRunImport, PostgresError> {
        Ok(GitHubRunImport {
            repository_id: self.repo_id,
            github_repository_id: i64_to_u64(self.github_repository_id, "GitHub repository id")?,
            run_count: i32_to_u32(self.run_count, "GitHub run import count")?,
            state: decode_enum(self.state)?,
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
