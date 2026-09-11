use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0047_retire_apply_changes_permission"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
UPDATE scope_repository_members SET permissions = permissions - 'can_apply_changes'
    WHERE permissions ? 'can_apply_changes';
UPDATE scope_repository_invites SET permissions = permissions - 'can_apply_changes'
    WHERE permissions ? 'can_apply_changes';
        "#,
            )
            .await?;
        Ok(())
    }
}
