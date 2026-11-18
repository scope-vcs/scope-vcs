use crate::{db::AdminStore, error::PostgresError};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, Statement, TransactionTrait};

impl AdminStore {
    pub async fn lock_repository_history_for_tests(
        &self,
    ) -> Result<DatabaseTransaction, PostgresError> {
        self.lock_tables_for_tests(&[
            "scope_logical_commits",
            "scope_file_changes",
            "scope_visibility_change_sets",
            "scope_visibility_changes",
        ])
        .await
    }

    pub async fn lock_aggregates_for_tests(&self) -> Result<DatabaseTransaction, PostgresError> {
        self.lock_tables_for_tests(&["scope_metadata_locks"]).await
    }

    pub async fn lock_waiters_for_tests(&self) -> Result<i64, PostgresError> {
        Ok(self
            .db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT count(*) AS waiting FROM pg_locks
                 WHERE NOT granted
                   AND database = (SELECT oid FROM pg_database WHERE datname = current_database())",
            ))
            .await?
            .ok_or_else(|| PostgresError::internal_message("lock waiter count missing"))?
            .try_get::<i64>("", "waiting")?)
    }

    pub async fn wait_for_aggregate_lock_waiter_for_tests(
        &self,
        held: &DatabaseTransaction,
    ) -> Result<(), PostgresError> {
        let blocker = held
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT pg_backend_pid() AS pid",
            ))
            .await?
            .ok_or_else(|| PostgresError::internal_message("lock holder pid missing"))?
            .try_get::<i32>("", "pid")?;
        loop {
            let waiting = self
                .db
                .query_one_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT EXISTS (
                        SELECT 1 FROM pg_locks
                        WHERE NOT granted AND locktype = 'relation'
                          AND database = (SELECT oid FROM pg_database WHERE datname = current_database())
                          AND relation = 'scope_metadata_locks'::regclass
                          AND $1 = ANY(pg_blocking_pids(pid))
                    ) AS waiting",
                    [blocker.into()],
                ))
                .await?
                .ok_or_else(|| PostgresError::internal_message("lock waiter state missing"))?
                .try_get::<bool>("", "waiting")?;
            if waiting {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
    }

    async fn lock_tables_for_tests(
        &self,
        tables: &[&str],
    ) -> Result<DatabaseTransaction, PostgresError> {
        let tx = self.db.begin().await?;
        tx.execute_unprepared(&format!(
            "LOCK TABLE {} IN ACCESS EXCLUSIVE MODE",
            tables.join(", ")
        ))
        .await?;
        Ok(tx)
    }
}
