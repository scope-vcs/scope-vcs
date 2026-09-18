use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0055_request_checks"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_request_check_evaluations (
                    request_id varchar NOT NULL
                        REFERENCES scope_requests(id) ON DELETE CASCADE,
                    head_oid varchar NOT NULL,
                    state varchar NOT NULL,
                    message text,
                    checks jsonb NOT NULL,
                    created_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    PRIMARY KEY (request_id, head_oid),
                    CONSTRAINT scope_request_check_evaluation_values CHECK (
                        length(head_oid) = 40 AND
                        state IN ('no-checks', 'awaiting-approval', 'started', 'configuration-error') AND
                        jsonb_typeof(checks) = 'array' AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                        ((state = 'configuration-error') = (length(btrim(coalesce(message, ''))) > 0)) AND
                        ((state IN ('awaiting-approval', 'started')) = (jsonb_array_length(checks) > 0))
                    )
                );

                ALTER TABLE scope_runs
                    DROP CONSTRAINT scope_runs_values;
                ALTER TABLE scope_runs
                    ADD CONSTRAINT scope_runs_values CHECK (
                        char_length(workflow_revision_digest) = 64 AND
                        workflow_revision_digest ~ '^[0-9A-Fa-f]+$' AND
                        (
                            (
                                source->>'kind' = 'ephemeral-git-bundle' AND
                                char_length(source#>>'{object,sha256}') = 64 AND
                                (source#>>'{object,sha256}') ~ '^[0-9A-Fa-f]+$' AND
                                char_length(source#>>'{object,git_oid}') = 40 AND
                                (source#>>'{object,git_oid}') ~ '^[0-9A-Fa-f]+$'
                            ) OR (
                                source->>'kind' = 'accepted-git-head' AND
                                length(btrim(source->>'repository_id')) > 0 AND
                                source->>'audience' IN ('Private', 'Public') AND
                                (source#>>'{head,push_sequence}')::numeric > 0 AND
                                (source#>>'{head,change_version}')::numeric > 0 AND
                                char_length(source#>>'{head,head_oid}') = 40 AND
                                (source#>>'{head,head_oid}') ~ '^[0-9A-Fa-f]+$' AND
                                char_length(source#>>'{head,frontier}') = 64 AND
                                (source#>>'{head,frontier}') ~ '^[0-9A-Fa-f]+$' AND
                                jsonb_typeof(source->'pack_spans') = 'array' AND
                                jsonb_array_length(source->'pack_spans') > 0 AND
                                ((source->'pack_spans')->(jsonb_array_length(source->'pack_spans') - 1)->>'last_sequence')::numeric =
                                    (source#>>'{head,push_sequence}')::numeric AND
                                ((source->'pack_spans')->(jsonb_array_length(source->'pack_spans') - 1)->>'head_oid') =
                                    (source#>>'{head,head_oid}')
                            )
                        ) AND
                        trigger IN ('manual', 'push-main', 'request') AND
                        state IN ('queued', 'dispatching', 'running', 'succeeded', 'failed', 'canceled', 'lost') AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                        ((state IN ('succeeded', 'failed', 'canceled', 'lost')) =
                            (completed_at_unix IS NOT NULL)) AND
                        (completed_at_unix IS NULL OR completed_at_unix = updated_at_unix) AND
                        (state <> 'canceled' OR cancellation_requested)
                    );
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Request checks are forward-only".into()))
    }
}
