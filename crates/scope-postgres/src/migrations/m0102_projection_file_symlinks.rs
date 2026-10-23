use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0102_projection_file_symlinks"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_projection_files
                    DROP CONSTRAINT scope_projection_file_values,
                    ADD CONSTRAINT scope_projection_file_values CHECK (
                        length(btrim(visibility)) > 0
                        AND size_bytes >= 0
                        AND git_file_mode IN ('100644', '100755', '120000')
                    );
                "#,
            )
            .await?;
        Ok(())
    }
}
