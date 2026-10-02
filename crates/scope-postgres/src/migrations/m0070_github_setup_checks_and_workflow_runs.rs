use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0070_github_setup_checks_and_workflow_runs"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- A connection test pushes main to scope/setup-check, which no
                -- request owns.
                ALTER TABLE scope_github_pushes
                    ALTER COLUMN request_id DROP NOT NULL,
                    DROP CONSTRAINT scope_github_push_values,
                    ADD CONSTRAINT scope_github_push_values CHECK (
                        ((request_id IS NOT NULL AND
                          ref = 'refs/heads/scope/requests/' || request_id) OR
                         (request_id IS NULL AND ref = 'refs/heads/scope/setup-check')) AND
                        sequence > 0 AND installation_id > 0 AND
                        github_repository_id > 0 AND
                        length(btrim(github_full_name)) > 0 AND
                        kind IN ('push', 'delete') AND
                        ((kind = 'push') = (target_oid IS NOT NULL)) AND
                        (target_oid IS NULL OR length(target_oid) = 40) AND
                        state IN ('queued', 'running', 'succeeded', 'failed') AND
                        ((state = 'running') = (lease_until_unix IS NOT NULL)) AND
                        ((state = 'running') = (claim_token IS NOT NULL)) AND
                        (state <> 'failed' OR last_error IS NOT NULL) AND
                        attempts >= 0 AND next_attempt_at_unix >= 0 AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix
                    );

                -- Each GitHub Actions workflow run files its jobs under its own
                -- check suite, which ties a check run to the branch it ran on.
                ALTER TABLE scope_github_check_runs
                    ADD COLUMN check_suite_id bigint,
                    ADD CONSTRAINT scope_github_check_run_suite
                        CHECK (check_suite_id IS NULL OR check_suite_id > 0);

                -- A repository's latest connection test, of the GitHub
                -- repository it was connected to when the test started.
                CREATE TABLE scope_github_setup_checks (
                    repo_id varchar PRIMARY KEY
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    github_repository_id bigint NOT NULL,
                    commit_oid varchar NOT NULL,
                    state varchar NOT NULL,
                    started_at_unix bigint NOT NULL,
                    finished_at_unix bigint,
                    last_error text,
                    CONSTRAINT scope_github_setup_check_values CHECK (
                        github_repository_id > 0 AND length(commit_oid) = 40 AND
                        state IN ('pushing', 'waiting', 'finished', 'failed') AND
                        ((state IN ('finished', 'failed')) = (finished_at_unix IS NOT NULL)) AND
                        ((state = 'failed') = (last_error IS NOT NULL)) AND
                        started_at_unix >= 0 AND
                        (finished_at_unix IS NULL OR finished_at_unix >= started_at_unix)
                    )
                );

                CREATE INDEX idx_scope_github_setup_checks_running
                    ON scope_github_setup_checks(started_at_unix)
                    WHERE state IN ('pushing', 'waiting');

                -- Workflow runs GitHub Actions reported for connected
                -- repositories, as the Runs page lists them. Like check runs,
                -- a run belongs to the GitHub repository that reported it.
                CREATE TABLE scope_github_workflow_runs (
                    github_run_id bigint PRIMARY KEY,
                    repo_id varchar NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    github_repository_id bigint NOT NULL,
                    workflow_name text NOT NULL,
                    head_branch text,
                    head_oid varchar NOT NULL,
                    event varchar NOT NULL,
                    status varchar NOT NULL,
                    conclusion varchar,
                    html_url text NOT NULL,
                    check_suite_id bigint,
                    run_started_at_unix bigint,
                    github_created_at_unix bigint NOT NULL,
                    github_updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_workflow_run_values CHECK (
                        github_run_id > 0 AND github_repository_id > 0 AND
                        length(btrim(workflow_name)) > 0 AND
                        length(head_oid) = 40 AND
                        length(btrim(event)) > 0 AND
                        status IN ('queued', 'in_progress', 'completed', 'waiting', 'requested', 'pending') AND
                        conclusion IN (
                            'success', 'neutral', 'skipped', 'failure', 'cancelled',
                            'timed_out', 'action_required', 'stale', 'startup_failure'
                        ) AND
                        ((status = 'completed') = (conclusion IS NOT NULL)) AND
                        (check_suite_id IS NULL OR check_suite_id > 0) AND
                        (run_started_at_unix IS NULL OR run_started_at_unix >= 0) AND
                        github_created_at_unix >= 0 AND github_updated_at_unix >= 0
                    )
                );

                CREATE INDEX idx_scope_github_workflow_runs_recent
                    ON scope_github_workflow_runs(
                        repo_id, github_repository_id,
                        coalesce(run_started_at_unix, github_updated_at_unix) DESC,
                        github_run_id DESC
                    );

                CREATE INDEX idx_scope_github_workflow_runs_branch
                    ON scope_github_workflow_runs(
                        repo_id, github_repository_id, head_branch, head_oid
                    );
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "GitHub setup checks and workflow runs are forward-only".into(),
        ))
    }
}
