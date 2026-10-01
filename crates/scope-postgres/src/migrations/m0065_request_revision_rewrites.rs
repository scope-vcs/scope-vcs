use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0065_request_revision_rewrites"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- Request pushes may now rebase or amend, and each revision may move
                -- the request base forward. Until now every push was a fast-forward
                -- and the base never moved, so existing revisions extend their old
                -- head and were recorded against their request's current base.
                ALTER TABLE scope_request_revisions
                    ADD COLUMN base_main_oid character varying,
                    ADD COLUMN rewrote_history boolean NOT NULL DEFAULT FALSE;
                UPDATE scope_request_revisions AS revision
                SET base_main_oid = request.base_main_oid
                FROM scope_requests AS request
                WHERE request.id = revision.request_id;
                ALTER TABLE scope_request_revisions
                    ALTER COLUMN base_main_oid SET NOT NULL,
                    ALTER COLUMN rewrote_history DROP DEFAULT,
                    ADD CONSTRAINT scope_request_revision_base
                        CHECK (length(base_main_oid::text) > 0);
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Request revision rewrites are forward-only".into(),
        ))
    }
}
