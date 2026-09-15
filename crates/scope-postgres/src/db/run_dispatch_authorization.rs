use super::RunStore;
use crate::error::PostgresError;
use scope_domain::runs::dispatch_authorization::{self, DispatchAuthorization};
use sea_orm::TransactionTrait;

impl RunStore {
    pub async fn authorize_dispatch_start(
        &self,
        attempt_id: &str,
        bootstrap_hash: &str,
        now_unix: u64,
    ) -> Result<DispatchAuthorization, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        // This context locks the job before reading cancellation, matching cancellation's lock order.
        let (run, job, attempt) =
            super::run_attempt_persistence::locked_heartbeat_context(&tx, attempt_id).await?;
        let authorization = dispatch_authorization::authorize_start(
            &run,
            &job,
            &attempt,
            bootstrap_hash,
            now_unix,
        )?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(authorization)
    }

    pub async fn authorize_dispatch_stop(
        &self,
        attempt_id: &str,
    ) -> Result<DispatchAuthorization, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (run, _, attempt) =
            super::run_attempt_persistence::locked_heartbeat_context(&tx, attempt_id).await?;
        let authorization = dispatch_authorization::authorize_stop(&run, &attempt)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(authorization)
    }
}
