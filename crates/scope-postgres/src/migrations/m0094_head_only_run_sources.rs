use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0094_head_only_run_sources"
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

                UPDATE scope_runs
                SET source = source - 'pack_spans'
                WHERE source ? 'pack_spans';

                UPDATE scope_outbox_jobs
                SET payload = payload - 'pack_spans'
                WHERE kind = 'push_main_trigger_evaluation' AND payload ? 'pack_spans';

                ALTER TABLE scope_runs ADD CONSTRAINT scope_runs_values CHECK (
                    char_length(workflow_revision_digest) = 64
                    AND workflow_revision_digest ~ '^[0-9A-Fa-f]+$'
                    AND (
                        (
                            source ->> 'kind' = 'ephemeral-git-bundle'
                            AND char_length(source #>> '{object,sha256}') = 64
                            AND source #>> '{object,sha256}' ~ '^[0-9A-Fa-f]+$'
                            AND char_length(source #>> '{object,git_oid}') = 40
                            AND source #>> '{object,git_oid}' ~ '^[0-9A-Fa-f]+$'
                        )
                        OR (
                            source ->> 'kind' = 'request-git-snapshot'
                            AND char_length(source #>> '{object,sha256}') = 64
                            AND source #>> '{object,sha256}' ~ '^[0-9A-Fa-f]+$'
                            AND char_length(source #>> '{object,git_oid}') = 40
                            AND source #>> '{object,git_oid}' ~ '^[0-9A-Fa-f]+$'
                            AND char_length(source ->> 'base_oid') = 40
                            AND source ->> 'base_oid' ~ '^[0-9A-Fa-f]+$'
                            AND trigger = 'request'
                        )
                        OR (
                            source ->> 'kind' = 'accepted-git-head'
                            AND length(btrim(source ->> 'repository_id')) > 0
                            AND (source #>> '{head,push_sequence}')::numeric > 0
                            AND (source #>> '{head,change_version}')::numeric > 0
                            AND char_length(source #>> '{head,head_oid}') = 40
                            AND source #>> '{head,head_oid}' ~ '^[0-9A-Fa-f]+$'
                            AND char_length(source #>> '{head,frontier}') = 64
                            AND source #>> '{head,frontier}' ~ '^[0-9A-Fa-f]+$'
                        )
                    )
                    AND trigger IN ('manual', 'push-main', 'request')
                    AND state IN (
                        'queued', 'dispatching', 'running', 'succeeded', 'failed',
                        'canceled', 'lost'
                    )
                    AND created_at_unix >= 0
                    AND updated_at_unix >= created_at_unix
                    AND (state IN ('succeeded', 'failed', 'canceled', 'lost'))
                        = (completed_at_unix IS NOT NULL)
                    AND (completed_at_unix IS NULL OR completed_at_unix = updated_at_unix)
                    AND (state <> 'canceled' OR cancellation_requested)
                );

                DROP TABLE scope_git_segment_references;

                ALTER TABLE scope_git_segment_uploads
                    DROP CONSTRAINT scope_git_segment_upload_state,
                    DROP CONSTRAINT scope_git_segment_upload_values;

                UPDATE scope_git_segment_uploads
                SET state = 'deleting'
                WHERE state = 'retained';

                ALTER TABLE scope_git_segment_uploads
                    ADD CONSTRAINT scope_git_segment_upload_state CHECK (
                        state IN ('uploading', 'ready', 'published', 'deleting', 'deleted')
                    ),
                    ADD CONSTRAINT scope_git_segment_upload_values CHECK (
                        length(btrim(segment_id)) > 0
                        AND length(btrim(object_key)) > 0
                        AND encoding_version > 0
                        AND created_at_unix >= 0
                        AND updated_at_unix >= created_at_unix
                        AND (sha256 IS NULL OR length(sha256) = 64)
                        AND (plaintext_bytes IS NULL OR plaintext_bytes >= 0)
                        AND (encrypted_bytes IS NULL OR encrypted_bytes >= 0)
                        AND (
                            state NOT IN ('ready', 'published')
                            OR (
                                sha256 IS NOT NULL
                                AND plaintext_bytes IS NOT NULL
                                AND encrypted_bytes IS NOT NULL
                            )
                        )
                    );
                "#,
            )
            .await?;
        Ok(())
    }
}
