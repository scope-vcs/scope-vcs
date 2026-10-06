use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0075_view_ids"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
TRUNCATE scope_repository_history_entries, scope_repository_history_payloads,
    scope_projection_files, scope_projection_read_models;

ALTER TABLE scope_visibility_changes DROP CONSTRAINT scope_visibility_change_values;
ALTER TABLE scope_requests DROP CONSTRAINT scope_request_identity_values;
ALTER TABLE scope_runs DROP CONSTRAINT scope_runs_values;

UPDATE scope_file_changes SET visibility = lower(visibility);
UPDATE scope_visibility_changes SET old_visibility = lower(old_visibility),
    new_visibility = lower(new_visibility);
UPDATE scope_requests SET audience = lower(audience);

ALTER TABLE scope_file_changes ADD CONSTRAINT scope_file_change_view
    CHECK (length(btrim(visibility)) > 0);
ALTER TABLE scope_visibility_changes ADD CONSTRAINT scope_visibility_change_values
    CHECK (ordinal >= 0 AND length(path) > 0 AND
        length(btrim(old_visibility)) > 0 AND length(btrim(new_visibility)) > 0 AND
        old_visibility <> new_visibility);
ALTER TABLE scope_projection_files DROP CONSTRAINT scope_projection_file_values;
ALTER TABLE scope_projection_files ADD CONSTRAINT scope_projection_file_values
    CHECK (length(btrim(visibility)) > 0 AND size_bytes >= 0 AND
        git_file_mode IN ('100644', '100755'));
ALTER TABLE scope_projection_read_models DROP CONSTRAINT scope_projection_read_model_values;
ALTER TABLE scope_projection_read_models ADD CONSTRAINT scope_projection_read_model_values
    CHECK (length(btrim(audience)) > 0 AND repo_version >= 0 AND
        folded_commits >= 0 AND folded_change_sets >= 0 AND
        (last_commit_id IS NULL) = (folded_commits = 0) AND
        (last_change_set_id IS NULL) = (folded_change_sets = 0) AND
        projected_commits >= 0 AND
        (last_projected_id IS NULL) = (projected_commits = 0) AND
        (head_oid IS NULL) = (projected_commits = 0) AND
        (head_oid IS NULL OR head_oid ~ '^[0-9a-f]{40}$') AND
        file_count >= 0 AND history_entries >= 0 AND
        (last_history_entry_id IS NULL) = (history_entries = 0));
ALTER TABLE scope_requests ADD CONSTRAINT scope_request_identity_values
    CHECK (name ~ '^[a-z0-9][a-z0-9-]{0,47}$' AND
        name NOT IN ('main', 'head', 'scope') AND
        length(btrim(audience)) > 0 AND
        author_role IN ('Public', 'Member', 'Owner'));

DROP INDEX idx_scope_requests_public_closed_queue;
DROP INDEX idx_scope_requests_public_open_queue;
DROP INDEX idx_scope_requests_public_search;
CREATE INDEX idx_scope_requests_public_closed_queue ON scope_requests
    (repo_id, COALESCE(closed_at_unix, merged_at_unix) DESC, id)
    WHERE audience = 'public' AND
        (closed_at_unix IS NOT NULL OR merged_at_unix IS NOT NULL);
CREATE INDEX idx_scope_requests_public_open_queue ON scope_requests
    (repo_id, submitted_at_unix, id)
    WHERE audience = 'public' AND submitted_at_unix IS NOT NULL AND
        closed_at_unix IS NULL AND merged_at_unix IS NULL;
CREATE INDEX idx_scope_requests_public_search ON scope_requests USING gin
    (title public.gin_trgm_ops, description_markdown public.gin_trgm_ops)
    WHERE audience = 'public';

UPDATE scope_repositories SET repo_config =
    jsonb_build_object(
        'kind', 'scope.repo-config', 'version', 2,
        'views', jsonb_build_array(
            jsonb_build_object('id','public','name','Public','includes',jsonb_build_array(),'readers','anyone'),
            jsonb_build_object('id','private','name','Private','includes','all','readers','members')),
        'files', jsonb_build_object(
            'default', lower(repo_config#>>'{visibility,default}'),
            'rules', COALESCE((
                SELECT jsonb_agg(jsonb_build_object('path', rule->'path',
                    'view', lower(rule->>'visibility')) ORDER BY ordinal)
                FROM jsonb_array_elements(repo_config#>'{visibility,rules}') WITH ORDINALITY AS rules(rule,ordinal)
            ), '[]'::jsonb)),
        'history', COALESCE(repo_config->'history', jsonb_build_object('rewrites',jsonb_build_array())))
    WHERE repo_config->>'version' = '1';
UPDATE scope_repositories SET policy =
    jsonb_build_object(
        'default', lower(policy->>'default_visibility'),
        'rules', COALESCE((
            SELECT jsonb_agg(jsonb_build_object('path', rule->'path',
                'view', lower(rule->>'visibility')) ORDER BY ordinal)
            FROM jsonb_array_elements(policy->'rules') WITH ORDINALITY AS rules(rule,ordinal)
        ), '[]'::jsonb))
    WHERE policy ? 'default_visibility';
UPDATE scope_repository_members SET permissions =
    permissions - 'can_read_private_files' || jsonb_build_object('view','private');
UPDATE scope_repository_invites SET permissions =
    permissions - 'can_read_private_files' || jsonb_build_object('view','private');
UPDATE scope_runs SET source = source - 'audience'
    WHERE source->>'kind' = 'accepted-git-head';
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
"#).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Named views are forward-only".into()))
    }
}
