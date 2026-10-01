use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0067_provider_neutral_request_checks"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- A request check names its provider. Every stored check so far
                -- is a native workflow, and native runs test the head itself.
                ALTER TABLE scope_request_check_evaluations
                    DROP CONSTRAINT scope_request_check_evaluation_values,
                    ADD COLUMN tested_oid varchar;
                UPDATE scope_request_check_evaluations
                   SET tested_oid = head_oid,
                       checks = (
                           SELECT coalesce(
                               jsonb_agg(
                                   jsonb_build_object('provider', 'native') || item.element
                                   ORDER BY item.position
                               ),
                               '[]'::jsonb
                           )
                             FROM jsonb_array_elements(checks)
                                  WITH ORDINALITY AS item(element, position)
                       );
                ALTER TABLE scope_request_check_evaluations
                    ALTER COLUMN tested_oid SET NOT NULL,
                    ADD CONSTRAINT scope_request_check_evaluation_values CHECK (
                        length(head_oid) = 40 AND
                        length(tested_oid) = 40 AND
                        state IN ('no-checks', 'awaiting-approval', 'started', 'configuration-error') AND
                        jsonb_typeof(checks) = 'array' AND
                        NOT checks @? '$[*] ? (!(@.provider == "native" || @.provider == "github"))' AND
                        (tested_oid = head_oid OR NOT checks @? '$[*] ? (@.provider == "native")') AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                        ((state = 'configuration-error') = (length(btrim(coalesce(message, ''))) > 0)) AND
                        ((state IN ('awaiting-approval', 'started')) = (jsonb_array_length(checks) > 0))
                    );

                -- Check runs GitHub reports for commits Scope pushed. Every run
                -- is kept: a re-run is a new row, and the latest to start decides.
                CREATE TABLE scope_github_check_runs (
                    github_check_run_id bigint PRIMARY KEY,
                    repo_id varchar NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    commit_oid varchar NOT NULL,
                    name text NOT NULL,
                    status varchar NOT NULL,
                    conclusion varchar,
                    details_url text,
                    started_at_unix bigint NOT NULL,
                    completed_at_unix bigint,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_check_run_values CHECK (
                        github_check_run_id > 0 AND
                        length(commit_oid) = 40 AND
                        length(btrim(name)) > 0 AND
                        status IN ('queued', 'in_progress', 'completed', 'waiting', 'requested', 'pending') AND
                        conclusion IN (
                            'success', 'neutral', 'skipped', 'failure', 'cancelled',
                            'timed_out', 'action_required', 'stale', 'startup_failure'
                        ) AND
                        ((status = 'completed') = (conclusion IS NOT NULL)) AND
                        started_at_unix >= 0 AND
                        (completed_at_unix IS NULL OR completed_at_unix >= 0) AND
                        updated_at_unix >= 0
                    )
                );

                CREATE INDEX idx_scope_github_check_runs_commit
                    ON scope_github_check_runs(
                        repo_id, commit_oid, name, started_at_unix DESC, github_check_run_id DESC
                    );
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Provider-neutral request checks are forward-only".into(),
        ))
    }
}
