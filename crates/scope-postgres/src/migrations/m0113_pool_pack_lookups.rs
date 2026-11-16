use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0113_pool_pack_lookups"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX scope_git_pool_packs_head
                    ON scope_git_pool_packs (repo_id, head_oid);
                CREATE INDEX scope_git_pool_packs_request_view
                    ON scope_git_pool_packs (request_id, view_id);
                CREATE INDEX scope_request_revisions_parent
                    ON scope_request_revisions (request_id, parent_request_id)
                    WHERE parent_request_id IS NOT NULL",
            )
            .await?;
        Ok(())
    }
}
