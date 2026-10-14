use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0090_job_traceparents"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_outbox_jobs ADD COLUMN traceparent text;
                ALTER TABLE scope_git_compaction_jobs ADD COLUMN traceparent text;
                ALTER TABLE scope_dependency_analysis_jobs ADD COLUMN traceparent text;
                ALTER TABLE scope_orphan_object_jobs ADD COLUMN traceparent text;
                ALTER TABLE scope_request_media_cleanup_jobs ADD COLUMN traceparent text;
                ALTER TABLE scope_request_media_processing_jobs ADD COLUMN traceparent text;
                "#,
            )
            .await?;
        Ok(())
    }
}
