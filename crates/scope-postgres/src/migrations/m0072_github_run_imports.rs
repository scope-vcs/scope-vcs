use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0072_github_run_imports"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- How many of GitHub's most recent workflow runs a repository
                -- imports when it connects. A repository without a row
                -- imports the default.
                CREATE TABLE scope_github_run_import_counts (
                    repo_id varchar PRIMARY KEY
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    run_count integer NOT NULL,
                    CONSTRAINT scope_github_run_import_count_values CHECK (
                        run_count BETWEEN 0 AND 1000
                    )
                );

                -- A repository's latest import of the GitHub repository it
                -- was connected to when the import was queued. A background
                -- pass claims it with a lease and tries a failed attempt again.
                CREATE TABLE scope_github_run_imports (
                    repo_id varchar PRIMARY KEY
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    github_repository_id bigint NOT NULL,
                    run_count integer NOT NULL,
                    state varchar NOT NULL,
                    attempts integer NOT NULL,
                    imported_count integer NOT NULL,
                    last_error text,
                    next_attempt_at_unix bigint NOT NULL,
                    lease_until_unix bigint,
                    claim_token varchar,
                    queued_at_unix bigint NOT NULL,
                    finished_at_unix bigint,
                    CONSTRAINT scope_github_run_import_values CHECK (
                        github_repository_id > 0 AND
                        run_count BETWEEN 1 AND 1000 AND
                        state IN ('queued', 'running', 'succeeded', 'failed') AND
                        ((state = 'running') = (lease_until_unix IS NOT NULL)) AND
                        ((state = 'running') = (claim_token IS NOT NULL)) AND
                        ((state IN ('succeeded', 'failed')) = (finished_at_unix IS NOT NULL)) AND
                        (state <> 'failed' OR last_error IS NOT NULL) AND
                        (state <> 'succeeded' OR last_error IS NULL) AND
                        attempts >= 0 AND
                        imported_count BETWEEN 0 AND run_count AND
                        next_attempt_at_unix >= 0 AND queued_at_unix >= 0 AND
                        (finished_at_unix IS NULL OR finished_at_unix >= queued_at_unix)
                    )
                );

                CREATE INDEX idx_scope_github_run_imports_due
                    ON scope_github_run_imports(next_attempt_at_unix)
                    WHERE state IN ('queued', 'running');
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("GitHub run imports are forward-only".into()))
    }
}
