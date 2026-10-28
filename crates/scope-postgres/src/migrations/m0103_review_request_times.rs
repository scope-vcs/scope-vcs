use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0103_review_request_times"
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
                    ADD COLUMN commented_at_unix bigint;

                UPDATE scope_github_pull_requests
                   SET commented_at_unix = updated_at_unix
                 WHERE commented_head_oid IS NOT NULL;

                ALTER TABLE scope_github_pull_requests
                    ADD CONSTRAINT scope_github_pull_request_review_request CHECK (
                        (commented_head_oid IS NULL AND commented_at_unix IS NULL)
                        OR (commented_head_oid IS NOT NULL AND commented_at_unix >= 0)
                    );

                ALTER TABLE scope_github_review_bot_statuses
                    DROP CONSTRAINT scope_github_review_bot_status_values,
                    ADD CONSTRAINT scope_github_review_bot_status_values CHECK (
                        length(btrim(login)) > 0
                        AND length(btrim(head_oid)) > 0
                        AND state IN ('not_started', 'working', 'commented', 'clear', 'timed_out')
                        AND updated_at_unix >= 0
                    );
                "#,
            )
            .await?;
        Ok(())
    }
}
