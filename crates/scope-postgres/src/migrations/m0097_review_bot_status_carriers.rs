use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0097_review_bot_status_carriers"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                DELETE FROM scope_github_review_bot_statuses;

                ALTER TABLE scope_github_review_bot_statuses
                    ADD COLUMN github_repository_id bigint NOT NULL,
                    ADD COLUMN pull_number bigint NOT NULL,
                    ADD CONSTRAINT scope_github_review_bot_status_carrier CHECK (
                        github_repository_id > 0 AND pull_number > 0
                    );
                "#,
            )
            .await?;
        Ok(())
    }
}
