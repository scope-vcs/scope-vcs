use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0073_github_workflow_jobs"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- The jobs of GitHub Actions workflow runs, as GitHub last
                -- reported each. A read replaces a stored job unless the stored
                -- one has come further: stage 0 waiting, 1 running, 2 completed,
                -- then how many of its steps finished and started.
                CREATE TABLE scope_github_workflow_jobs (
                    github_job_id bigint PRIMARY KEY,
                    repo_id varchar NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    github_repository_id bigint NOT NULL,
                    github_run_id bigint NOT NULL,
                    run_attempt integer NOT NULL,
                    name text NOT NULL,
                    status varchar NOT NULL,
                    conclusion varchar,
                    started_at_unix bigint,
                    completed_at_unix bigint,
                    html_url text NOT NULL,
                    steps jsonb NOT NULL,
                    stage smallint NOT NULL,
                    steps_completed integer NOT NULL,
                    steps_started integer NOT NULL,
                    CONSTRAINT scope_github_workflow_job_values CHECK (
                        github_job_id > 0 AND github_repository_id > 0 AND
                        github_run_id > 0 AND run_attempt > 0 AND
                        length(btrim(name)) > 0 AND
                        status IN ('queued', 'in_progress', 'completed', 'waiting', 'requested', 'pending') AND
                        conclusion IN (
                            'success', 'neutral', 'skipped', 'failure', 'cancelled',
                            'timed_out', 'action_required', 'stale', 'startup_failure'
                        ) AND
                        ((status = 'completed') = (conclusion IS NOT NULL)) AND
                        (started_at_unix IS NULL OR started_at_unix >= 0) AND
                        (completed_at_unix IS NULL OR completed_at_unix >= 0) AND
                        jsonb_typeof(steps) = 'array' AND
                        stage IN (0, 1, 2) AND ((stage = 2) = (status = 'completed')) AND
                        steps_completed >= 0 AND steps_started >= steps_completed
                    )
                );

                CREATE INDEX idx_scope_github_workflow_jobs_run
                    ON scope_github_workflow_jobs(
                        repo_id, github_repository_id, github_run_id, run_attempt
                    );

                -- The end of a finished job's log, read from GitHub the first
                -- time someone opens it. A finished job's log does not change.
                CREATE TABLE scope_github_workflow_job_logs (
                    github_job_id bigint PRIMARY KEY
                        REFERENCES scope_github_workflow_jobs(github_job_id) ON DELETE CASCADE,
                    log_text text NOT NULL,
                    truncated boolean NOT NULL,
                    stored_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_workflow_job_log_values CHECK (stored_at_unix >= 0)
                );

                -- When Scope last read a run's jobs from GitHub, and for which
                -- attempt, so opening a run reads them only when they may be
                -- behind.
                ALTER TABLE scope_github_workflow_runs
                    ADD COLUMN jobs_read_attempt integer,
                    ADD COLUMN jobs_read_at_unix bigint,
                    ADD CONSTRAINT scope_github_workflow_run_jobs_read CHECK (
                        (jobs_read_attempt IS NULL) = (jobs_read_at_unix IS NULL) AND
                        (jobs_read_attempt IS NULL OR jobs_read_attempt > 0) AND
                        (jobs_read_at_unix IS NULL OR jobs_read_at_unix >= 0)
                    );
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "GitHub workflow jobs are forward-only".into(),
        ))
    }
}
