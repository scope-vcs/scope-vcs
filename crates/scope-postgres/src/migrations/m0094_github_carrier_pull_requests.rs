use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0094_github_carrier_pull_requests"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_github_connections
                    ADD COLUMN open_review_bot_pull_requests boolean NOT NULL DEFAULT false,
                    ADD COLUMN review_request_comment text,
                    ADD CONSTRAINT scope_github_connection_review_request_comment CHECK (
                        review_request_comment IS NULL
                        OR (length(btrim(review_request_comment)) > 0
                            AND char_length(review_request_comment) <= 500)
                    );

                CREATE TABLE scope_github_pull_requests (
                    request_id character varying PRIMARY KEY
                        REFERENCES scope_requests(id) ON DELETE CASCADE,
                    repo_id character varying NOT NULL,
                    installation_id bigint NOT NULL,
                    github_repository_id bigint NOT NULL,
                    github_full_name character varying NOT NULL,
                    intent character varying NOT NULL,
                    intent_base character varying,
                    intent_head_oid character varying,
                    intent_sequence bigint NOT NULL,
                    number bigint,
                    url text,
                    base character varying,
                    state character varying,
                    commented_head_oid character varying,
                    job_state character varying NOT NULL,
                    attempts integer NOT NULL,
                    next_attempt_at_unix bigint NOT NULL,
                    lease_until_unix bigint,
                    claim_token character varying,
                    last_error text,
                    created_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_pull_request_values CHECK (
                        installation_id > 0
                        AND github_repository_id > 0
                        AND length(btrim(github_full_name)) > 0
                        AND intent IN ('open', 'close')
                        AND ((intent = 'open') = (intent_base IS NOT NULL))
                        AND ((intent = 'open') = (intent_head_oid IS NOT NULL))
                        AND intent_sequence > 0
                        AND (number IS NULL) = (url IS NULL)
                        AND (number IS NULL) = (base IS NULL)
                        AND (number IS NULL) = (state IS NULL)
                        AND (number IS NULL OR number > 0)
                        AND (state IS NULL OR state IN ('open', 'closed', 'closed_on_github'))
                        AND job_state IN ('queued', 'running', 'done', 'failed')
                        AND ((job_state = 'running') = (lease_until_unix IS NOT NULL))
                        AND ((job_state = 'running') = (claim_token IS NOT NULL))
                        AND (job_state <> 'failed' OR last_error IS NOT NULL)
                        AND attempts >= 0
                        AND next_attempt_at_unix >= 0
                        AND created_at_unix >= 0
                        AND updated_at_unix >= created_at_unix
                    )
                );

                CREATE INDEX idx_scope_github_pull_requests_due
                    ON scope_github_pull_requests (next_attempt_at_unix)
                    WHERE job_state IN ('queued', 'running');

                CREATE INDEX idx_scope_github_pull_requests_repository
                    ON scope_github_pull_requests (github_repository_id, number);
                "#,
            )
            .await?;
        Ok(())
    }
}
