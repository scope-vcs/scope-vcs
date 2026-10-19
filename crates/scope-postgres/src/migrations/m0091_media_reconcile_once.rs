use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0091_media_reconcile_once"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_request_media_cleanup_jobs ADD COLUMN reconciled_at_unix bigint;
                ALTER TABLE scope_request_media_abandoned_objects ADD COLUMN reconciled_at_unix bigint;
                ALTER TABLE scope_request_media_processing_objects ADD COLUMN reconciled_at_unix bigint;
                CREATE INDEX scope_request_media_cleanup_reconcile_idx
                    ON scope_request_media_cleanup_jobs (completed_at_unix, attachment_id)
                    WHERE state = 'Completed' AND reconciled_at_unix IS NULL;
                CREATE INDEX scope_request_media_abandoned_reconcile_idx
                    ON scope_request_media_abandoned_objects (deleted_at_unix, object_key)
                    WHERE deleted_at_unix IS NOT NULL AND reconciled_at_unix IS NULL;
                CREATE INDEX scope_request_media_processing_objects_reconcile_idx
                    ON scope_request_media_processing_objects (updated_at_unix, object_key)
                    WHERE state = 'Deleted' AND manifest_id IS NULL AND reconciled_at_unix IS NULL;
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Migration(
            "media reconciliation tracking is forward-only".into(),
        ))
    }
}
