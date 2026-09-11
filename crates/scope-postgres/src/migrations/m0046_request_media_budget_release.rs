use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0046_request_media_budget_release"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
ALTER TABLE scope_request_media_attachments
    ADD COLUMN budget_released_at_unix bigint,
    ADD CONSTRAINT scope_request_media_budget_release_time CHECK (
        budget_released_at_unix IS NULL OR budget_released_at_unix >= 0
    );
UPDATE scope_request_media_attachments attachment
    SET budget_released_at_unix = cleanup.completed_at_unix
    FROM scope_request_media_cleanup_jobs cleanup
    WHERE attachment.id = cleanup.attachment_id AND cleanup.state = 'Completed';
        "#,
            )
            .await?;
        Ok(())
    }
}
