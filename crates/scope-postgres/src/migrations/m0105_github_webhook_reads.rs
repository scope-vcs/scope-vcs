use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0105_github_webhook_reads"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE scope_github_check_refreshes
                 ADD COLUMN lease_until_unix bigint NOT NULL DEFAULT 0
                 CHECK (lease_until_unix >= 0);
             ALTER TABLE scope_github_workflow_run_reads
                 ADD COLUMN read_id bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
                 ADD COLUMN generation bigint NOT NULL DEFAULT 0 CHECK (generation >= 0),
                 ADD COLUMN lease_until_unix bigint NOT NULL DEFAULT 0
                 CHECK (lease_until_unix >= 0);
             ALTER TABLE scope_github_workflow_job_reads
                 ADD COLUMN run_attempt integer NOT NULL DEFAULT 1 CHECK (run_attempt > 0),
                 ADD COLUMN lease_until_unix bigint NOT NULL DEFAULT 0
                 CHECK (lease_until_unix >= 0);",
            )
            .await?;
        Ok(())
    }
}
