use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0048_run_attempt_active_state_indexes"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The attempt state machine has no 'leased' state: an attempt is
        // 'dispatching' until the runner starts it and 'running' until it
        // completes. The original predicates named 'leased', so 'dispatching'
        // attempts escaped both the one-active-attempt-per-job uniqueness
        // rule and the lease-expiry scan.
        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX idx_scope_run_attempts_active;
                 DROP INDEX idx_scope_run_attempts_expiring;
                 CREATE UNIQUE INDEX idx_scope_run_attempts_active
                     ON scope_run_attempts (run_id, job_key)
                     WHERE state IN ('dispatching', 'running');
                 CREATE INDEX idx_scope_run_attempts_expiring
                     ON scope_run_attempts (lease_expires_at_unix, id)
                     WHERE state IN ('dispatching', 'running');",
            )
            .await?;
        Ok(())
    }
}
