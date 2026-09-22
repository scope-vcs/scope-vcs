use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0059_worker_history_permissions"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // The pending ledger entry forces the paused maintenance cutover. Its
        // apply wrapper refreshes runtime-roles.mjs grants before writers reopen.
        Ok(())
    }
}
