use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0043_retire_git_manifests"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
                LOCK TABLE scope_git_heads, scope_runs, scope_outbox_jobs,
                    scope_object_references, scope_orphan_object_jobs
                    IN ACCESS EXCLUSIVE MODE;

                -- Keep a durable deletion job for every former manifest owner before
                -- dropping its metadata. Keys use the runtime's compact JSON encoding.
                WITH retired AS (
                    SELECT manifest_object_key::jsonb AS content_ref,
                        manifest_sha256 AS sha256, head_oid AS git_oid,
                        manifest_size_bytes AS size_bytes
                    FROM scope_git_heads
                    UNION ALL
                    SELECT source#>'{head,manifest,content_ref}',
                        source#>>'{head,manifest,sha256}',
                        source#>>'{head,manifest,git_oid}',
                        (source#>>'{head,manifest,size_bytes}')::bigint
                    FROM scope_runs
                    WHERE source->>'kind' = 'accepted-git-head'
                    UNION ALL
                    SELECT payload#>'{head,manifest,content_ref}',
                        payload#>>'{head,manifest,sha256}',
                        payload#>>'{head,manifest,git_oid}',
                        (payload#>>'{head,manifest,size_bytes}')::bigint
                    FROM scope_outbox_jobs
                    WHERE kind = 'push_main_trigger_evaluation'
                      AND payload#>'{head,manifest}' IS NOT NULL
                    UNION ALL
                    SELECT object_key::jsonb, sha256, git_oid, size_bytes
                    FROM scope_orphan_object_jobs
                    WHERE object_key::jsonb ? 'GitManifestSha256'
                      AND completed_at_unix IS NULL
                    UNION ALL
                    -- Includes references whose original owner has already gone away.
                    SELECT object_key::jsonb, object_key::jsonb->>'GitManifestSha256',
                        repeat('0', 40), 0
                    FROM scope_object_references
                    WHERE object_key::jsonb ? 'GitManifestSha256'
                ), objects AS (
                    SELECT DISTINCT ON (content_ref)
                        replace(content_ref::text, ': ', ':') AS object_key,
                        sha256, git_oid, size_bytes
                    FROM retired
                    WHERE content_ref ? 'GitManifestSha256'
                    ORDER BY content_ref, size_bytes DESC
                )
                INSERT INTO scope_orphan_object_jobs (
                    object_key, generation, sha256, git_oid, size_bytes,
                    attempts, next_run_at_unix, last_error, completed_at_unix,
                    created_at_unix, updated_at_unix
                )
                SELECT object_key, 'm0043_retire_git_manifests', sha256, git_oid,
                    size_bytes, 0, 0, NULL, NULL, 0, 0
                FROM objects
                ON CONFLICT (object_key) DO UPDATE SET
                    generation = EXCLUDED.generation,
                    attempts = 0, next_run_at_unix = 0, last_error = NULL,
                    completed_at_unix = NULL;

                -- Earlier migrations wrote spaced JSON keys. Coalesce those jobs
                -- onto the compact key used by cleanup claims and completion.
                DELETE FROM scope_orphan_object_jobs old
                USING scope_orphan_object_jobs canonical
                WHERE old.object_key::jsonb ? 'GitManifestSha256'
                  AND old.object_key::jsonb = canonical.object_key::jsonb
                  AND old.object_key <> canonical.object_key
                  AND canonical.object_key = replace(canonical.object_key::jsonb::text, ': ', ':');

                DELETE FROM scope_object_references
                WHERE object_key::jsonb ? 'GitManifestSha256';

                -- Preserve the exact signed-push/cache identity. Older heads may
                -- carry hashes from another manifest encoding; never recompute them.
                UPDATE scope_runs SET source = jsonb_set(source, '{head,frontier}',
                    source#>'{head,manifest,sha256}') #- '{head,manifest}'
                WHERE source->>'kind' = 'accepted-git-head';
                UPDATE scope_outbox_jobs SET payload = jsonb_set(payload, '{head,frontier}',
                    payload#>'{head,manifest,sha256}') #- '{head,manifest}'
                WHERE kind = 'push_main_trigger_evaluation'
                  AND payload#>'{head,manifest}' IS NOT NULL;

                ALTER TABLE scope_git_heads
                    RENAME COLUMN manifest_sha256 TO frontier_digest;
                ALTER TABLE scope_git_heads
                    DROP CONSTRAINT scope_git_head_values,
                    DROP COLUMN manifest_object_key,
                    DROP COLUMN manifest_size_bytes,
                    ADD CONSTRAINT scope_git_head_values CHECK (
                        push_sequence >= 0 AND change_version >= 0
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
                        trigger IN ('manual', 'push-main') AND
                        state IN ('queued', 'dispatching', 'running', 'succeeded', 'failed', 'canceled', 'lost') AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                        ((state IN ('succeeded', 'failed', 'canceled', 'lost')) =
                            (completed_at_unix IS NOT NULL)) AND
                        (completed_at_unix IS NULL OR completed_at_unix = updated_at_unix) AND
                        (state <> 'canceled' OR cancellation_requested)
                    );
        "#).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Git manifest retirement is forward-only".into(),
        ))
    }
}
