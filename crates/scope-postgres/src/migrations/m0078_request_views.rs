use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0078_request_views"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
UPDATE scope_logical_commits
SET origin = jsonb_build_object('RequestMerge', jsonb_build_object(
    'request_id', origin->'PublicRequestMerge'->'request_id',
    'view', 'public',
    'base_oid', origin->'PublicRequestMerge'->'public_base_oid',
    'parent_oids', origin->'PublicRequestMerge'->'public_parent_oids',
    'request_head_oid', origin->'PublicRequestMerge'->'request_head_oid',
    'preserve_commits', origin->'PublicRequestMerge'->'preserve_public_commits',
    'commits', origin->'PublicRequestMerge'->'commits'))
WHERE origin ? 'PublicRequestMerge';
"#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Request views are forward-only".into()))
    }
}
