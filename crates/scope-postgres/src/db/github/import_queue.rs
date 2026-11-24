use super::super::integer_columns::u64_to_i64;
use super::read_queue::{GitHubJobOutcome, prefixed_columns};
use crate::error::PostgresError;
use scope_domain::github::import::github_import_error;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};

pub(in crate::db) struct GitHubImportQueue {
    pub table: &'static str,
    pub columns: &'static str,
    pub label: &'static str,
}

impl GitHubImportQueue {
    pub(in crate::db) async fn load<R: FromQueryResult>(
        &self,
        conn: &impl ConnectionTrait,
        repo_id: &str,
    ) -> Result<Option<R>, PostgresError> {
        let Self { table, columns, .. } = self;
        Ok(R::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!("SELECT {columns} FROM {table} WHERE repo_id = $1"),
            [repo_id.into()],
        ))
        .one(conn)
        .await?)
    }

    pub(in crate::db) async fn claim_due<R: FromQueryResult>(
        &self,
        conn: &impl ConnectionTrait,
        claim_token: &str,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<R>, PostgresError> {
        let Self {
            table,
            columns,
            label,
        } = self;
        Ok(R::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "UPDATE {table} import
                    SET state = 'running', lease_until_unix = $2, claim_token = $3,
                        next_attempt_at_unix = $2, attempts = import.attempts + 1
                   FROM (SELECT repo_id
                           FROM {table}
                          WHERE (state = 'queued' AND next_attempt_at_unix <= $1)
                             OR (state = 'running' AND lease_until_unix <= $1)
                          ORDER BY next_attempt_at_unix, repo_id
                          LIMIT $4
                          FOR UPDATE SKIP LOCKED) due
                  WHERE import.repo_id = due.repo_id
              RETURNING {}",
                prefixed_columns(columns, "import.")
            ),
            [
                u64_to_i64(now_unix, &format!("{label} claim time"))?.into(),
                u64_to_i64(lease_until_unix, &format!("{label} lease"))?.into(),
                claim_token.into(),
                u64_to_i64(limit, &format!("{label} batch size"))?.into(),
            ],
        ))
        .all(conn)
        .await?)
    }

    pub(in crate::db) async fn lock_claimed<R: FromQueryResult>(
        &self,
        conn: &impl ConnectionTrait,
        repo_id: &str,
        claim_token: &str,
    ) -> Result<Option<R>, PostgresError> {
        let Self { table, columns, .. } = self;
        Ok(R::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {}
                   FROM {table} import
                   JOIN scope_github_connections connection
                     ON connection.repo_id = import.repo_id
                    AND connection.github_repository_id = import.github_repository_id
                    AND connection.status = 'Connected'
                  WHERE import.repo_id = $1 AND import.claim_token = $2
                    AND import.state = 'running'
                    AND import.lease_until_unix > extract(epoch FROM now())::bigint
                    FOR UPDATE OF import FOR SHARE OF connection",
                prefixed_columns(columns, "import.")
            ),
            [repo_id.into(), claim_token.into()],
        ))
        .one(conn)
        .await?)
    }

    pub(in crate::db) async fn finish<T>(
        &self,
        conn: &impl ConnectionTrait,
        repo_id: &str,
        claim_token: &str,
        outcome: &GitHubJobOutcome<T>,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let Self { table, label, .. } = self;
        let now = u64_to_i64(now_unix, &format!("{label} time"))?;
        let (state, error, retry_at, finished) = match outcome {
            GitHubJobOutcome::Succeeded(_) => ("succeeded", None, None, Some(now)),
            GitHubJobOutcome::Failed {
                error,
                retry_at_unix: Some(retry_at),
            } => (
                "queued",
                Some(github_import_error(error)),
                Some(u64_to_i64(*retry_at, &format!("{label} retry time"))?),
                None,
            ),
            GitHubJobOutcome::Failed {
                error,
                retry_at_unix: None,
            } => ("failed", Some(github_import_error(error)), None, Some(now)),
        };
        let result = conn
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                format!(
                    "UPDATE {table}
                        SET state = $3, last_error = $4,
                            next_attempt_at_unix = coalesce($5, next_attempt_at_unix),
                            finished_at_unix = CASE WHEN $6::bigint IS NULL THEN NULL
                                               ELSE greatest($6, queued_at_unix) END,
                            lease_until_unix = NULL, claim_token = NULL
                      WHERE repo_id = $1 AND claim_token = $2 AND state = 'running'"
                ),
                [
                    repo_id.into(),
                    claim_token.into(),
                    state.into(),
                    error.into(),
                    retry_at.into(),
                    finished.into(),
                ],
            ))
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub(in crate::db) async fn delete(
        &self,
        conn: &impl ConnectionTrait,
        repo_id: &str,
    ) -> Result<(), PostgresError> {
        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!("DELETE FROM {} WHERE repo_id = $1", self.table),
            [repo_id.into()],
        ))
        .await?;
        Ok(())
    }
}
