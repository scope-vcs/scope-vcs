use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0045_cache_upload_cleanup_leases"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
ALTER TABLE scope_cache_uploads
    ADD COLUMN cleanup_generation bigint NOT NULL DEFAULT 0,
    ADD COLUMN cleanup_lease_expires_at_unix bigint;
UPDATE scope_cache_uploads SET cleanup_lease_expires_at_unix = 0 WHERE state = 'deleting';
ALTER TABLE scope_cache_uploads ADD CONSTRAINT scope_cache_upload_cleanup_values CHECK (
    cleanup_generation >= 0 AND (
        (state = 'deleting' AND cleanup_lease_expires_at_unix IS NOT NULL AND cleanup_lease_expires_at_unix >= 0)
        OR (state <> 'deleting' AND cleanup_lease_expires_at_unix IS NULL)
    )
);
CREATE INDEX idx_scope_cache_uploads_cleanup_lease ON scope_cache_uploads (cleanup_lease_expires_at_unix, upload_id)
    WHERE state = 'deleting';
        "#).await?;
        Ok(())
    }
}
