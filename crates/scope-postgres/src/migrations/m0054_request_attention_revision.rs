use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0054_request_attention_revision"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
ALTER TABLE scope_request_attention_states
    ADD COLUMN revision bigint NOT NULL DEFAULT 1 CHECK (revision >= 1);
ALTER TABLE scope_request_attention_states ALTER COLUMN revision DROP DEFAULT;
                "#,
            )
            .await?;
        Ok(())
    }
}
