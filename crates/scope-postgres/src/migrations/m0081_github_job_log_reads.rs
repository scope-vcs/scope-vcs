use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0081_github_job_log_reads"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- Finished jobs whose log Scope still has to read from GitHub,
                -- queued when a failed job finishes or a viewer opens a log
                -- Scope does not have, and retried while GitHub publishes it.
                CREATE TABLE scope_github_workflow_job_log_reads (
                    github_job_id bigint PRIMARY KEY
                        REFERENCES scope_github_workflow_jobs(github_job_id) ON DELETE CASCADE,
                    attempts integer NOT NULL,
                    next_attempt_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_workflow_job_log_read_values CHECK (
                        attempts >= 0 AND next_attempt_at_unix >= 0
                    )
                );

                CREATE INDEX idx_scope_github_workflow_job_log_reads_due
                    ON scope_github_workflow_job_log_reads(next_attempt_at_unix);
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "GitHub job log reads are forward-only".into(),
        ))
    }
}
