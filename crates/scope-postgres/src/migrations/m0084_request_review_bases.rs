use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0084_request_review_bases"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
            ALTER TABLE scope_requests RENAME COLUMN base_main_oid TO main_ancestor_oid;
            ALTER TABLE scope_request_revisions RENAME COLUMN base_main_oid TO main_ancestor_oid;
            ALTER TABLE scope_requests
                ADD COLUMN base_kind text NOT NULL DEFAULT 'Main' CHECK (base_kind = 'Main'),
                ADD COLUMN review_base_oid text,
                ADD COLUMN base_version bigint NOT NULL DEFAULT 1 CHECK (base_version > 0);
            ALTER TABLE scope_request_revisions
                ADD COLUMN base_kind text NOT NULL DEFAULT 'Main' CHECK (base_kind = 'Main'),
                ADD COLUMN review_base_oid text;
            UPDATE scope_requests SET review_base_oid = main_ancestor_oid;
            UPDATE scope_request_revisions SET review_base_oid = main_ancestor_oid;
            ALTER TABLE scope_requests ALTER COLUMN review_base_oid SET NOT NULL;
            ALTER TABLE scope_request_revisions ALTER COLUMN review_base_oid SET NOT NULL;
            ALTER TABLE scope_requests ALTER COLUMN base_kind DROP DEFAULT;
            ALTER TABLE scope_request_revisions ALTER COLUMN base_kind DROP DEFAULT;
            ALTER TABLE scope_requests ALTER COLUMN base_version DROP DEFAULT;
            "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Migration(
            "request review bases are forward-only".to_string(),
        ))
    }
}
