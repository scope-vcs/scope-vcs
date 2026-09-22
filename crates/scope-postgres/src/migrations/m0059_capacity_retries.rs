use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0059_capacity_retries"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
            ALTER TABLE scope_run_jobs
                ADD COLUMN capacity_retry_first_rejected_at_unix bigint,
                ADD COLUMN capacity_retry_rejections integer NOT NULL DEFAULT 0,
                ADD COLUMN capacity_retry_next_attempt_at_unix bigint,
                ADD CONSTRAINT scope_run_jobs_capacity_retry_values CHECK (
                    (capacity_retry_first_rejected_at_unix IS NULL) = (capacity_retry_rejections = 0)
                    AND capacity_retry_rejections BETWEEN 0 AND 4
                    AND (capacity_retry_first_rejected_at_unix IS NULL OR capacity_retry_first_rejected_at_unix >= created_at_unix)
                    AND (capacity_retry_next_attempt_at_unix IS NULL OR (
                        state = 'queued' AND capacity_retry_rejections BETWEEN 1 AND 3
                        AND capacity_retry_next_attempt_at_unix > capacity_retry_first_rejected_at_unix
                        AND capacity_retry_next_attempt_at_unix < capacity_retry_first_rejected_at_unix + 120
                    ))
                );
            CREATE INDEX idx_scope_run_jobs_capacity_retry_due
                ON scope_run_jobs(capacity_retry_next_attempt_at_unix)
                WHERE state = 'queued' AND capacity_retry_next_attempt_at_unix IS NOT NULL;
        "#).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Capacity retries are forward-only".into()))
    }
}
