use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0108_github_pull_request_merges"
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
                    ADD COLUMN merge_commit_oid character varying,
                    ADD COLUMN merged_head_oid character varying,
                    DROP CONSTRAINT scope_github_pull_request_values,
                    ADD CONSTRAINT scope_github_pull_request_values CHECK (
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
                        AND (state IS NULL
                            OR state IN ('open', 'closed', 'closed_on_github', 'merged'))
                        AND job_state IN ('queued', 'running', 'done', 'failed')
                        AND ((job_state = 'running') = (lease_until_unix IS NOT NULL))
                        AND ((job_state = 'running') = (claim_token IS NOT NULL))
                        AND (job_state <> 'failed' OR last_error IS NOT NULL)
                        AND attempts >= 0
                        AND next_attempt_at_unix >= 0
                        AND created_at_unix >= 0
                        AND updated_at_unix >= created_at_unix
                    ),
                    ADD CONSTRAINT scope_github_pull_request_merge CHECK (
                        ((state IS NOT DISTINCT FROM 'merged') = (merge_commit_oid IS NOT NULL))
                        AND ((state IS NOT DISTINCT FROM 'merged') = (merged_head_oid IS NOT NULL))
                        AND (merge_commit_oid IS NULL OR length(merge_commit_oid) > 0)
                        AND (merged_head_oid IS NULL OR length(merged_head_oid) > 0)
                    );

                ALTER TABLE scope_git_heads
                    ADD COLUMN main_pushed_by_user_id character varying
                        REFERENCES scope_users(id) ON DELETE SET NULL;
                "#,
            )
            .await?;
        Ok(())
    }
}
