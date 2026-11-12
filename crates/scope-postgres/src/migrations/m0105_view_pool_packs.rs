use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0105_view_pool_packs"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE scope_git_pool_packs
                ADD COLUMN view_id text,
                ADD CONSTRAINT scope_git_pool_pack_view CHECK (
                    view_id IS NULL OR (request_id IS NOT NULL AND length(view_id) > 0)
                )",
            )
            .await?;
        Ok(())
    }
}
