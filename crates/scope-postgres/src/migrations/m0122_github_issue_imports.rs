use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0122_github_issue_imports"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_github_issue_imports (
                    repo_id character varying PRIMARY KEY
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    github_repository_id bigint NOT NULL,
                    view_id character varying NOT NULL,
                    state character varying NOT NULL,
                    attempts integer NOT NULL,
                    imported_count integer NOT NULL,
                    already_imported_count integer NOT NULL,
                    skipped_issues jsonb NOT NULL,
                    read_through_created_at_unix bigint NOT NULL,
                    read_through_issue_number bigint NOT NULL,
                    last_error text,
                    next_attempt_at_unix bigint NOT NULL,
                    lease_until_unix bigint,
                    claim_token character varying,
                    queued_at_unix bigint NOT NULL,
                    finished_at_unix bigint,
                    CONSTRAINT scope_github_issue_import_values CHECK (
                        github_repository_id > 0
                        AND length(view_id) > 0
                        AND state IN ('queued', 'running', 'succeeded', 'failed')
                        AND (state = 'running') = (lease_until_unix IS NOT NULL)
                        AND (state = 'running') = (claim_token IS NOT NULL)
                        AND (state IN ('succeeded', 'failed')) = (finished_at_unix IS NOT NULL)
                        AND (state <> 'failed' OR last_error IS NOT NULL)
                        AND (state <> 'succeeded' OR last_error IS NULL)
                        AND attempts >= 0
                        AND imported_count >= 0
                        AND already_imported_count >= 0
                        AND jsonb_typeof(skipped_issues) = 'array'
                        AND read_through_created_at_unix >= 0
                        AND read_through_issue_number >= 0
                        AND next_attempt_at_unix >= 0
                        AND queued_at_unix >= 0
                        AND (finished_at_unix IS NULL OR finished_at_unix >= queued_at_unix)
                    )
                );

                CREATE INDEX idx_scope_github_issue_imports_due
                    ON scope_github_issue_imports (next_attempt_at_unix)
                    WHERE state IN ('queued', 'running');
                "#,
            )
            .await?;
        Ok(())
    }
}
