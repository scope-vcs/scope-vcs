use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0110_request_diff_renames"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DELETE FROM scope_request_revision_diff_stats;
                 ALTER TABLE scope_request_revision_diff_files
                    ADD COLUMN renamed_from text,
                    ADD COLUMN old_file_deletions bigint,
                    ADD COLUMN new_file_additions bigint,
                    ADD CONSTRAINT scope_request_revision_diff_file_renames CHECK (
                        (renamed_from IS NULL OR length(renamed_from) > 1)
                        AND (renamed_from IS NOT NULL
                            OR (old_file_deletions IS NULL AND new_file_additions IS NULL))
                        AND (old_file_deletions IS NULL OR old_file_deletions >= 0)
                        AND (new_file_additions IS NULL OR new_file_additions >= 0)
                    )",
            )
            .await?;
        Ok(())
    }
}
