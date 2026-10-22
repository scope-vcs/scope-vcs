use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0096_github_review_bot_gate"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_github_review_bots
                    ADD COLUMN required boolean NOT NULL DEFAULT false;

                ALTER TABLE scope_github_pull_requests
                    ADD COLUMN review_bots_due_at_unix bigint NOT NULL DEFAULT 0,
                    ADD CONSTRAINT scope_github_pull_request_review_bots_due CHECK (
                        review_bots_due_at_unix >= 0
                    );

                CREATE INDEX idx_scope_github_pull_requests_review_bots_due
                    ON scope_github_pull_requests (review_bots_due_at_unix)
                    WHERE state = 'open';

                CREATE TABLE scope_github_review_bot_statuses (
                    request_id character varying NOT NULL,
                    login text NOT NULL,
                    head_oid character varying NOT NULL,
                    state character varying NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_review_bot_statuses_pkey
                        PRIMARY KEY (request_id, login),
                    CONSTRAINT fk_scope_github_review_bot_statuses_request
                        FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE,
                    CONSTRAINT scope_github_review_bot_status_values CHECK (
                        length(btrim(login)) > 0
                        AND length(btrim(head_oid)) > 0
                        AND state IN ('not_started', 'working', 'commented', 'clear')
                        AND updated_at_unix >= 0
                    )
                );
                "#,
            )
            .await?;
        Ok(())
    }
}
