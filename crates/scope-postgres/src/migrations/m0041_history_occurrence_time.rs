use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0041_history_occurrence_time"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Existing events have no recorded time. Rebuild disposable views while
        // leaving that unknown source metadata null.
        manager
            .get_connection()
            .execute_unprepared(
                "
             ALTER TABLE scope_logical_commits ADD COLUMN occurred_at_unix BIGINT;
             ALTER TABLE scope_visibility_change_sets ADD COLUMN occurred_at_unix BIGINT;
             DELETE FROM scope_repository_history_views;",
            )
            .await?;
        Ok(())
    }
}
