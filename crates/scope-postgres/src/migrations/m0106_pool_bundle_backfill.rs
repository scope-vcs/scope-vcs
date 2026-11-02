use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0106_pool_bundle_backfill"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE scope_pool_bundle_backfill_lease (
                singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
                token text,
                available_at_unix bigint NOT NULL DEFAULT 0 CHECK (available_at_unix >= 0)
            );
            INSERT INTO scope_pool_bundle_backfill_lease (singleton) VALUES (true);
            CREATE TABLE scope_pool_bundle_backfill_failures (
                source_id text PRIMARY KEY,
                request_id character varying NOT NULL REFERENCES scope_requests(id) ON DELETE CASCADE,
                reason text NOT NULL CHECK (length(reason) > 0),
                attempted_at_unix bigint NOT NULL CHECK (attempted_at_unix >= 0),
                retry_at_unix bigint NOT NULL CHECK (retry_at_unix >= attempted_at_unix)
            );
            CREATE INDEX scope_git_pool_packs_backfill_head
                ON scope_git_pool_packs (request_id, head_oid);"
        ).await?;
        Ok(())
    }
}
