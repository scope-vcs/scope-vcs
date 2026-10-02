use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0071_public_check_commits"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- GitHub tests a public contribution as a check commit: the
                -- head merged onto private main. The two commits it merges from
                -- are kept so the push can build the same commit again. Every
                -- evaluation so far tests its head.
                ALTER TABLE scope_request_check_evaluations
                    ADD COLUMN check_private_main_oid varchar,
                    ADD COLUMN check_public_base_oid varchar,
                    ADD CONSTRAINT scope_request_check_evaluation_check_commit CHECK (
                        (check_private_main_oid IS NULL) = (check_public_base_oid IS NULL) AND
                        (check_private_main_oid IS NULL) = (tested_oid = head_oid) AND
                        (check_private_main_oid IS NULL OR (
                            length(check_private_main_oid) = 40 AND
                            length(check_public_base_oid) = 40
                        ))
                    );
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Public check commits are forward-only".into(),
        ))
    }
}
