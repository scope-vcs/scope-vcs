use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;
impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0106_git_fetch_advertisements"
    }
}
#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE scope_git_fetch_advertisements (
                repo_id text NOT NULL REFERENCES scope_repositories(id) ON DELETE CASCADE,
                generation text NOT NULL,
                view_id text NOT NULL,
                reader_id text NOT NULL,
                head_oid text NOT NULL,
                requests jsonb,
                expires_at_unix bigint NOT NULL,
                PRIMARY KEY (repo_id, generation, view_id, reader_id, head_oid)
            );
            CREATE INDEX scope_git_fetch_advertisements_expiry ON scope_git_fetch_advertisements(expires_at_unix)"
        ).await?;
        Ok(())
    }
}
