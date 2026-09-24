use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0062_request_run_source_base"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_runs DROP CONSTRAINT scope_runs_values;

                -- The idempotency key records request ID and exact check head.
                -- A request's base stays pinned across later pushes, so old checks
                -- can be rebuilt without changing their source snapshot object.
                UPDATE scope_runs AS run
                SET source = jsonb_build_object(
                    'kind', 'request-git-snapshot',
                    'object', run.source->'object',
                    'base_oid', request.base_main_oid
                )
                FROM scope_requests AS request
                WHERE run.trigger = 'request'
                  AND run.source->>'kind' = 'ephemeral-git-bundle'
                  AND request.audience = 'Private'
                  AND request.repo_id = run.repo_id
                  AND split_part(run.idempotency_key, ':', 1) = 'request'
                  AND split_part(run.idempotency_key, ':', 2) = request.id
                  AND split_part(run.idempotency_key, ':', 3) = run.source#>>'{object,git_oid}'
                  AND char_length(request.base_main_oid) = 40
                  AND request.base_main_oid ~ '^[0-9A-Fa-f]+$';

                ALTER TABLE scope_runs ADD CONSTRAINT scope_runs_values CHECK (
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
                            source->>'kind' = 'request-git-snapshot' AND
                            char_length(source#>>'{object,sha256}') = 64 AND
                            (source#>>'{object,sha256}') ~ '^[0-9A-Fa-f]+$' AND
                            char_length(source#>>'{object,git_oid}') = 40 AND
                            (source#>>'{object,git_oid}') ~ '^[0-9A-Fa-f]+$' AND
                            char_length(source->>'base_oid') = 40 AND
                            (source->>'base_oid') ~ '^[0-9A-Fa-f]+$' AND
                            trigger = 'request'
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
        Err(DbErr::Custom(
            "Request run source base is forward-only".into(),
        ))
    }
}
