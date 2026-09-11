use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0048_request_ref_cleanup"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
CREATE TABLE scope_request_ref_cleanup_jobs (
    id text PRIMARY KEY,
    repo_id text NOT NULL,
    incarnation_id text NOT NULL,
    request_id text NOT NULL,
    request_name text NOT NULL,
    head_oid text NOT NULL,
    created_at_unix bigint NOT NULL CHECK (created_at_unix >= 0),
    next_run_at_unix bigint NOT NULL CHECK (next_run_at_unix >= 0),
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error text
);
CREATE INDEX idx_scope_request_ref_cleanup_ready ON scope_request_ref_cleanup_jobs (next_run_at_unix, id);
        "#).await?;
        Ok(())
    }
}
