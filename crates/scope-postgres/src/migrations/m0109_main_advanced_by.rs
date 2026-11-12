use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0109_main_advanced_by"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE scope_git_heads
                    RENAME COLUMN main_pushed_by_user_id TO main_advanced_by_user_id;
                 ALTER TABLE scope_git_heads
                    RENAME CONSTRAINT scope_git_heads_main_pushed_by_user_id_fkey
                    TO scope_git_heads_main_advanced_by_user_id_fkey;
                 ALTER TABLE scope_git_heads
                    ADD COLUMN main_advance_kind character varying NOT NULL DEFAULT 'Push'
                        CHECK (main_advance_kind IN ('Push', 'NativeMerge'))",
            )
            .await?;
        Ok(())
    }
}
