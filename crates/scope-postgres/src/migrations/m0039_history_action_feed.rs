use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0039_history_action_feed"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // History is disposable. Rebuild action rows from source records at the next read.
        manager
            .get_connection()
            .execute_unprepared(
                r#"

            DELETE FROM scope_repository_history_views;
            ALTER TABLE scope_repository_history_views ADD COLUMN history_version TEXT NOT NULL;
            DROP INDEX idx_scope_repository_history_entries_source;
            ALTER TABLE scope_repository_history_entries
                ADD UNIQUE (repo_id, audience, source_id);
        "#,
            )
            .await?;
        Ok(())
    }
}
