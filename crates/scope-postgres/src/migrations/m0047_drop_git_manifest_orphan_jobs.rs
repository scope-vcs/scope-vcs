use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0047_drop_git_manifest_orphan_jobs"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The runtime no longer has a GitManifestSha256 content reference, so
        // the cleanup queue cannot decode the drain jobs m0043 enqueued. Drop
        // them; pre-alpha object stores are reset rather than drained.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                DELETE FROM scope_orphan_object_jobs
                WHERE object_key::jsonb ? 'GitManifestSha256';
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "git manifest orphan job removal is forward-only".into(),
        ))
    }
}
