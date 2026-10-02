use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0069_github_request_checks"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- The check names GitHub must pass before a request merges, in
                -- the order maintainers listed them.
                CREATE TABLE scope_github_required_checks (
                    repo_id varchar NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    name text NOT NULL,
                    position integer NOT NULL,
                    PRIMARY KEY (repo_id, name),
                    CONSTRAINT scope_github_required_check_values CHECK (
                        length(btrim(name)) > 0 AND name = btrim(name) AND
                        char_length(name) <= 255 AND position >= 0
                    )
                );

                -- Pushes and deletions of request branches on GitHub. The
                -- request id is not a reference: a deleted draft's branch still
                -- has to be removed after its request row is gone.
                CREATE TABLE scope_github_pushes (
                    id varchar PRIMARY KEY,
                    repo_id varchar NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    request_id varchar NOT NULL,
                    ref varchar NOT NULL,
                    target_oid varchar,
                    kind varchar NOT NULL,
                    state varchar NOT NULL,
                    attempts integer NOT NULL,
                    next_attempt_at_unix bigint NOT NULL,
                    lease_until_unix bigint,
                    claim_token varchar,
                    last_error text,
                    created_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_push_values CHECK (
                        ref = 'refs/heads/scope/requests/' || request_id AND
                        kind IN ('push', 'delete') AND
                        ((kind = 'push') = (target_oid IS NOT NULL)) AND
                        (target_oid IS NULL OR length(target_oid) = 40) AND
                        state IN ('queued', 'running', 'succeeded', 'failed') AND
                        ((state = 'running') = (lease_until_unix IS NOT NULL)) AND
                        ((state = 'running') = (claim_token IS NOT NULL)) AND
                        (state <> 'failed' OR last_error IS NOT NULL) AND
                        attempts >= 0 AND next_attempt_at_unix >= 0 AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix
                    )
                );

                CREATE INDEX idx_scope_github_pushes_due
                    ON scope_github_pushes(next_attempt_at_unix)
                    WHERE state IN ('queued', 'running');

                CREATE INDEX idx_scope_github_pushes_ref
                    ON scope_github_pushes(repo_id, ref, created_at_unix);

                CREATE INDEX idx_scope_github_pushes_request
                    ON scope_github_pushes(request_id, created_at_unix DESC);

                -- A check run belongs to the GitHub repository that reported
                -- it, so a Scope repository reconnected to another GitHub
                -- repository never counts the old one's runs. Nothing has
                -- stored check runs before this migration.
                DELETE FROM scope_github_check_runs;
                DROP INDEX idx_scope_github_check_runs_commit;
                ALTER TABLE scope_github_check_runs
                    ADD COLUMN github_repository_id bigint NOT NULL,
                    ADD CONSTRAINT scope_github_check_run_repository CHECK (
                        github_repository_id > 0
                    );
                CREATE INDEX idx_scope_github_check_runs_commit
                    ON scope_github_check_runs(
                        repo_id, github_repository_id, commit_oid, name,
                        github_check_run_id DESC
                    );

                -- Reads of GitHub's check runs for a tested commit. Every read
                -- takes the next number before it asks GitHub, and its answer
                -- is stored only when no later read was stored first. The
                -- reconciler reads a commit again at next_read_at_unix.
                CREATE TABLE scope_github_check_refreshes (
                    repo_id varchar NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    github_repository_id bigint NOT NULL,
                    commit_oid varchar NOT NULL,
                    next_read_at_unix bigint NOT NULL,
                    started_reads bigint NOT NULL,
                    applied_read bigint NOT NULL,
                    -- When the stored read started; NULL before any.
                    applied_read_started_at_unix bigint,
                    PRIMARY KEY (repo_id, github_repository_id, commit_oid),
                    CONSTRAINT scope_github_check_refresh_values CHECK (
                        github_repository_id > 0 AND length(commit_oid) = 40 AND
                        next_read_at_unix >= 0 AND
                        applied_read >= 0 AND started_reads >= applied_read AND
                        ((applied_read = 0) = (applied_read_started_at_unix IS NULL))
                    )
                );

                -- A check delivery names a commit; Scope looks up whether any
                -- evaluation tests it.
                CREATE INDEX idx_scope_request_check_evaluations_tested_oid
                    ON scope_request_check_evaluations(tested_oid);
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "GitHub request checks are forward-only".into(),
        ))
    }
}
