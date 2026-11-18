use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0115_github_webhook_deliveries"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE scope_github_webhook_deliveries (
                    delivery_id character varying NOT NULL,
                    event character varying NOT NULL,
                    payload bytea NOT NULL,
                    attempts integer NOT NULL,
                    next_attempt_at_unix bigint NOT NULL,
                    received_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_webhook_deliveries_pkey PRIMARY KEY (delivery_id),
                    CONSTRAINT scope_github_webhook_delivery_values CHECK (
                        delivery_id <> ''
                        AND event <> ''
                        AND attempts >= 0
                        AND next_attempt_at_unix >= 0
                        AND received_at_unix >= 0
                    )
                );
                CREATE INDEX idx_scope_github_webhook_deliveries_due
                    ON scope_github_webhook_deliveries (next_attempt_at_unix)",
            )
            .await?;
        Ok(())
    }
}
