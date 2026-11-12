use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0085_request_check_identity"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
            ALTER TABLE scope_request_check_evaluations
                ADD COLUMN id text,
                ADD COLUMN revision_id text,
                ADD COLUMN base_version bigint,
                ADD COLUMN review_base_oid text,
                ADD COLUMN github_repository_id bigint;
            UPDATE scope_request_check_evaluations evaluation SET
                revision_id = revision.id,
                review_base_oid = revision.review_base_oid,
                base_version = request.base_version
            FROM scope_requests request, scope_request_revisions revision
            WHERE request.id = evaluation.request_id
              AND revision.id = (
                SELECT candidate.id FROM scope_request_revisions candidate
                WHERE candidate.request_id = evaluation.request_id
                  AND candidate.new_head_oid = evaluation.head_oid
                ORDER BY candidate.position DESC, candidate.id DESC LIMIT 1
              );
            DELETE FROM scope_request_check_evaluations WHERE revision_id IS NULL;
            UPDATE scope_request_check_evaluations evaluation SET github_repository_id = connection.github_repository_id
                FROM scope_requests request, scope_github_connections connection
                WHERE request.id = evaluation.request_id AND connection.repo_id = request.repo_id
                  AND evaluation.checks @> '[{"provider":"github"}]'::jsonb;
            UPDATE scope_request_check_evaluations evaluation SET
                id = 'evaluation_' || encode(sha256(
                    convert_to(request_id, 'UTF8') || decode('00', 'hex') ||
                    convert_to(revision_id, 'UTF8') || decode('00', 'hex') ||
                    convert_to(head_oid, 'UTF8') || decode('00', 'hex') ||
                    convert_to(review_base_oid, 'UTF8') || decode('00', 'hex') ||
                    convert_to(tested_oid, 'UTF8') || decode('00', 'hex') ||
                    int8send(coalesce(github_repository_id, 0)) ||
                    CASE WHEN check_private_main_oid IS NULL THEN ''::bytea ELSE
                        convert_to(check_private_main_oid, 'UTF8') || decode('00', 'hex') ||
                        convert_to(check_public_base_oid, 'UTF8') || decode('00', 'hex') END ||
                    coalesce((SELECT string_agg(
                        convert_to(item->>'provider', 'UTF8') || decode('00', 'hex') ||
                        CASE WHEN item->>'provider' = 'native' THEN
                            convert_to(item->>'workflow_path', 'UTF8') || decode('00', 'hex') ||
                            convert_to(item->>'workflow_revision_digest', 'UTF8') || decode('00', 'hex')
                        ELSE convert_to(item->>'name', 'UTF8') || decode('00', 'hex') END,
                        ''::bytea ORDER BY ordinal)
                        FROM jsonb_array_elements(evaluation.checks) WITH ORDINALITY AS entries(item, ordinal)), ''::bytea)
                ), 'hex');
            ALTER TABLE scope_request_check_evaluations
                DROP CONSTRAINT scope_request_check_evaluations_pkey,
                ALTER COLUMN id SET NOT NULL,
                ALTER COLUMN revision_id SET NOT NULL,
                ALTER COLUMN base_version SET NOT NULL,
                ALTER COLUMN review_base_oid SET NOT NULL,
                ADD PRIMARY KEY (id),
                ADD CONSTRAINT scope_request_check_identity_values CHECK (
                    length(id) > 0 AND length(revision_id) > 0 AND
                    base_version > 0 AND length(review_base_oid) = 40 AND
                    (github_repository_id IS NULL OR github_repository_id > 0)
                );
            ALTER TABLE scope_requests ADD COLUMN current_check_evaluation_id text
                REFERENCES scope_request_check_evaluations(id) ON DELETE SET NULL;
            UPDATE scope_requests request SET current_check_evaluation_id = evaluation.id
                FROM scope_request_check_evaluations evaluation
                WHERE evaluation.request_id = request.id
                  AND evaluation.head_oid = request.head_oid
                  AND evaluation.review_base_oid = request.review_base_oid
                  AND evaluation.state <> 'no-checks'
                  AND evaluation.revision_id = (
                    SELECT revision.id FROM scope_request_revisions revision
                    WHERE revision.request_id = request.id
                    ORDER BY revision.position DESC, revision.id DESC LIMIT 1
                  );
            ALTER TABLE scope_request_check_evaluations
                DROP CONSTRAINT scope_request_check_evaluation_values,
                ADD CONSTRAINT scope_request_check_evaluation_values CHECK (
                    length(head_oid) = 40 AND length(tested_oid) = 40 AND
                    state IN ('no-checks', 'awaiting-approval', 'started', 'configuration-error') AND
                    jsonb_typeof(checks) = 'array' AND
                    NOT checks @? '$[*] ? (!(@.provider == "native" || @.provider == "github"))' AND
                    (tested_oid = head_oid OR NOT checks @? '$[*] ? (@.provider == "native")') AND
                    created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                    ((state = 'configuration-error') = (length(btrim(coalesce(message, ''))) > 0)) AND
                    (state <> 'no-checks' OR jsonb_array_length(checks) = 0) AND
                    (state NOT IN ('awaiting-approval', 'started') OR jsonb_array_length(checks) > 0)
                );
            CREATE INDEX idx_scope_request_check_evaluations_revision
                ON scope_request_check_evaluations(request_id, revision_id);
        "#).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Request check identities are forward-only".into(),
        ))
    }
}
