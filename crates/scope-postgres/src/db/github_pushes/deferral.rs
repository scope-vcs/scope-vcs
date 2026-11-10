use crate::{
    db::{
        RequestStore,
        integer_columns::{i64_to_u64, u64_to_i64},
    },
    error::PostgresError,
};
use scope_domain::requests::github_branch_deletion_recheck_at;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "defer_github_branch_deletion"))]
    pub async fn defer_github_branch_deletion(
        &self,
        id: &str,
        claim_token: &str,
        reason: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await?;
        let Some(claimed) = tx
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT created_at_unix FROM scope_github_pushes
                  WHERE id = $1 AND claim_token = $2 AND state = 'running' AND kind = 'delete'
                    FOR UPDATE",
                [id.into(), claim_token.into()],
            ))
            .await?
        else {
            return Ok(());
        };
        let queued_at_unix = i64_to_u64(
            claimed.try_get("", "created_at_unix")?,
            "GitHub push queue time",
        )?;
        let recheck_at_unix = github_branch_deletion_recheck_at(queued_at_unix, now_unix);
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_github_pushes
                SET state = 'queued', lease_until_unix = NULL, claim_token = NULL,
                    attempts = attempts - 1, next_attempt_at_unix = $2, last_error = $3,
                    updated_at_unix = greatest($4, updated_at_unix)
              WHERE id = $1",
            [
                id.into(),
                u64_to_i64(recheck_at_unix, "GitHub push recheck time")?.into(),
                reason.into(),
                u64_to_i64(now_unix, "GitHub push time")?.into(),
            ],
        ))
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
