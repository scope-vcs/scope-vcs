use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0119_request_asks"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_requests
                    ADD COLUMN asker_github_user_id bigint,
                    ADD COLUMN github_issue_number bigint,
                    ADD COLUMN ask_source_url text,
                    ADD COLUMN ask_text_markdown text,
                    ADD COLUMN asked_at_unix bigint,
                    ADD COLUMN close_reason character varying,
                    ADD CONSTRAINT fk_scope_requests_github_asker
                        FOREIGN KEY (asker_github_user_id)
                        REFERENCES scope_github_accounts(github_user_id) ON DELETE RESTRICT,
                    ADD CONSTRAINT scope_request_ask CHECK (
                        (
                            asker_github_user_id IS NULL
                            AND github_issue_number IS NULL
                            AND ask_source_url IS NULL
                            AND ask_text_markdown IS NULL
                            AND asked_at_unix IS NULL
                        )
                        OR (
                            asker_github_user_id IS NOT NULL
                            AND github_issue_number > 0
                            AND length(btrim(ask_source_url)) > 0
                            AND ask_text_markdown IS NOT NULL
                            AND asked_at_unix >= 0
                            AND asked_at_unix <= created_at_unix
                        )
                    ),
                    ADD CONSTRAINT scope_request_close_reason CHECK (
                        (
                            close_reason IS NULL
                            AND (closed_at_unix IS NULL OR github_issue_number IS NULL)
                        )
                        OR (
                            close_reason IN ('completed', 'not_planned', 'duplicate')
                            AND closed_at_unix IS NOT NULL
                            AND github_issue_number IS NOT NULL
                        )
                    ),
                    DROP CONSTRAINT scope_request_submission_coherence,
                    ADD CONSTRAINT scope_request_submission_coherence CHECK (
                        (closed_at_unix IS NULL OR merged_at_unix IS NULL)
                        AND (closed_at_unix IS NOT NULL OR closed_by_user_id IS NULL)
                        AND (
                            (
                                submitted_at_unix IS NULL
                                AND merged_at_unix IS NULL
                                AND (closed_at_unix IS NULL OR github_issue_number IS NOT NULL)
                            )
                            OR (
                                submitted_at_unix IS NOT NULL
                                AND (closed_at_unix IS NULL OR closed_at_unix >= submitted_at_unix)
                                AND (merged_at_unix IS NULL OR merged_at_unix >= submitted_at_unix)
                            )
                        )
                    );

                CREATE UNIQUE INDEX idx_scope_requests_github_issue
                    ON scope_requests (repo_id, github_issue_number)
                    WHERE github_issue_number IS NOT NULL;
                "#,
            )
            .await?;
        Ok(())
    }
}
