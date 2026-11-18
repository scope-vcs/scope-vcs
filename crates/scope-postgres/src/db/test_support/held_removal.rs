use super::super::{AdminStore, acquire_aggregate_lock, entities};
use crate::error::PostgresError;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, EntityTrait,
    Statement, TransactionTrait,
};
use std::{sync::Arc, time::Duration};

pub struct HeldRemoval {
    tx: DatabaseTransaction,
    pid: i32,
    db: Arc<DatabaseConnection>,
}

impl AdminStore {
    pub async fn hold_repository_member_removal_for_tests(
        &self,
        repo_id: &str,
        user_id: &str,
    ) -> Result<HeldRemoval, PostgresError> {
        let tx = self.db.begin().await?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        entities::repository_member::Entity::delete_by_id((
            repo_id.to_string(),
            user_id.to_string(),
        ))
        .exec(&tx)
        .await?;
        self.held(tx).await
    }

    pub async fn hold_request_invitee_removal_for_tests(
        &self,
        request_id: &str,
        user_id: &str,
    ) -> Result<HeldRemoval, PostgresError> {
        let tx = self.db.begin().await?;
        acquire_aggregate_lock(&tx, "request", request_id).await?;
        entities::request_invitee::Entity::delete_by_id((
            request_id.to_string(),
            user_id.to_string(),
        ))
        .exec(&tx)
        .await?;
        self.held(tx).await
    }

    async fn held(&self, tx: DatabaseTransaction) -> Result<HeldRemoval, PostgresError> {
        let pid = tx
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT pg_backend_pid() AS pid",
            ))
            .await?
            .ok_or_else(|| PostgresError::internal_message("backend pid missing"))?
            .try_get("", "pid")?;
        Ok(HeldRemoval {
            tx,
            pid,
            db: Arc::clone(&self.db),
        })
    }
}

impl HeldRemoval {
    pub async fn wait_for_waiter(&self) -> Result<(), PostgresError> {
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let waiting = self
                    .db
                    .query_one_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        "SELECT pid FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) LIMIT 1",
                        [self.pid.into()],
                    ))
                    .await?;
                if waiting.is_some() {
                    return Ok(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .map_err(|_| PostgresError::internal_message("nothing waited for the held removal"))?
    }

    pub async fn commit(self) -> Result<(), PostgresError> {
        Ok(self.tx.commit().await?)
    }
}
