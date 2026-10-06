use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0074_history_path_indexes"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- Public request checks read the history of the paths a push
                -- changes, so those reads stay proportional to the paths
                -- rather than to the repository's whole history.
                CREATE INDEX idx_scope_file_changes_path
                    ON scope_file_changes(repo_id, path);
                CREATE INDEX idx_scope_visibility_changes_path
                    ON scope_visibility_changes(repo_id, path);
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "history path indexes are forward-only".into(),
        ))
    }
}
