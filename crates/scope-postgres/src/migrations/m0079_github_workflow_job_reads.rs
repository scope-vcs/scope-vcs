use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0079_github_workflow_job_reads"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
CREATE TABLE scope_github_workflow_job_reads (
    read_id bigint GENERATED ALWAYS AS IDENTITY,
    repo_id varchar NOT NULL REFERENCES scope_repositories(id) ON DELETE CASCADE,
    github_repository_id bigint NOT NULL,
    github_run_id bigint NOT NULL,
    attempts integer NOT NULL,
    generation bigint NOT NULL,
    next_attempt_at_unix bigint NOT NULL,
    PRIMARY KEY (repo_id, github_repository_id, github_run_id),
    CONSTRAINT scope_github_workflow_job_read_values CHECK (
        github_repository_id > 0 AND github_run_id > 0 AND
        attempts >= 0 AND generation >= 0 AND next_attempt_at_unix >= 0
    )
);
CREATE INDEX idx_scope_github_workflow_job_reads_due
    ON scope_github_workflow_job_reads(next_attempt_at_unix);
"#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE scope_github_workflow_job_reads")
            .await?;
        Ok(())
    }
}
