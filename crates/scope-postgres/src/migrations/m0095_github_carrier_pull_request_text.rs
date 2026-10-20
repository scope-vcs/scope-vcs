use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0095_github_carrier_pull_request_text"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_github_pull_requests
                    ADD COLUMN written_text_sha256 character varying,
                    ADD CONSTRAINT scope_github_pull_request_written_text CHECK (
                        written_text_sha256 IS NULL
                        OR (number IS NOT NULL AND length(written_text_sha256) = 64)
                    );
                "#,
            )
            .await?;
        Ok(())
    }
}
