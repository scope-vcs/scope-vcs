use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0086_private_request_stacks"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
            ALTER TABLE scope_requests
                DROP CONSTRAINT scope_requests_base_kind_check,
                ADD COLUMN parent_request_id text,
                ADD COLUMN parent_revision_id text,
                ADD COLUMN parent_head_oid text,
                ADD COLUMN stack_status text NOT NULL DEFAULT 'main',
                ADD COLUMN stack_depth integer NOT NULL DEFAULT 1,
                ADD CONSTRAINT scope_requests_base_values CHECK (
                    (base_kind = 'Main' AND parent_request_id IS NULL AND
                        parent_revision_id IS NULL AND parent_head_oid IS NULL AND
                        stack_depth = 1 AND stack_status IN ('main', 'integration_conflict')) OR
                    (base_kind = 'Request' AND parent_request_id IS NOT NULL AND
                        parent_revision_id IS NOT NULL AND parent_head_oid IS NOT NULL AND
                        parent_request_id <> id AND length(parent_request_id) > 0 AND
                        length(parent_revision_id) > 0 AND length(parent_head_oid) = 40 AND
                        review_base_oid = parent_head_oid AND stack_depth BETWEEN 2 AND 33 AND
                        stack_status IN ('waiting_for_parent', 'parent_extended', 'parent_rewritten',
                            'parent_closed', 'ancestor_blocked', 'integration_conflict'))
                ),
                ADD CONSTRAINT fk_scope_requests_parent_revision
                    FOREIGN KEY (parent_request_id, parent_revision_id)
                    REFERENCES scope_request_revisions(request_id, id);
            ALTER TABLE scope_request_revisions
                DROP CONSTRAINT scope_request_revisions_base_kind_check,
                ADD COLUMN base_version bigint CHECK (base_version > 0),
                ADD COLUMN parent_request_id text,
                ADD COLUMN parent_revision_id text,
                ADD COLUMN parent_head_oid text,
                ADD CONSTRAINT scope_request_revisions_base_values CHECK (
                    (base_kind = 'Main' AND parent_request_id IS NULL AND
                        parent_revision_id IS NULL AND parent_head_oid IS NULL) OR
                    (base_kind = 'Request' AND parent_request_id IS NOT NULL AND
                        parent_revision_id IS NOT NULL AND parent_head_oid IS NOT NULL AND
                        parent_request_id <> request_id AND length(parent_request_id) > 0 AND
                        length(parent_revision_id) > 0 AND length(parent_head_oid) = 40 AND
                        review_base_oid = parent_head_oid)
                ),
                ADD CONSTRAINT fk_scope_request_revisions_parent_revision
                    FOREIGN KEY (parent_request_id, parent_revision_id)
                    REFERENCES scope_request_revisions(request_id, id);
            WITH revision_bases AS (
                SELECT revision.id, revision.request_id, revision.position, revision.review_base_oid,
                    request.base_version,
                    lead(revision.review_base_oid, 1, request.review_base_oid) OVER (
                        PARTITION BY revision.request_id ORDER BY revision.position, revision.id
                    ) AS next_review_base_oid
                FROM scope_request_revisions revision
                JOIN scope_requests request ON request.id = revision.request_id
            ), revision_versions AS (
                SELECT id, greatest(1, base_version - sum(
                    CASE WHEN review_base_oid <> next_review_base_oid THEN 1 ELSE 0 END
                ) OVER (
                    PARTITION BY request_id ORDER BY position DESC, id DESC
                    ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                )) AS base_version
                FROM revision_bases
            )
            UPDATE scope_request_revisions revision SET base_version = version.base_version
                FROM revision_versions version WHERE version.id = revision.id;
            ALTER TABLE scope_request_revisions ALTER COLUMN base_version SET NOT NULL;
            ALTER TABLE scope_requests
                ALTER COLUMN stack_status DROP DEFAULT,
                ALTER COLUMN stack_depth DROP DEFAULT;
            CREATE INDEX idx_scope_requests_active_parent
                ON scope_requests(repo_id, parent_request_id)
                WHERE base_kind = 'Request' AND closed_at_unix IS NULL AND merged_at_unix IS NULL;
            CREATE TABLE scope_request_revision_context_receipts (
                request_id text NOT NULL REFERENCES scope_requests(id) ON DELETE CASCADE,
                operation_key text NOT NULL,
                payload_digest text NOT NULL,
                revision_id text NOT NULL,
                created_at_unix bigint NOT NULL,
                PRIMARY KEY (request_id, operation_key),
                CONSTRAINT scope_request_revision_context_receipts_values CHECK (
                    length(operation_key) > 0 AND length(payload_digest) = 64 AND
                    length(revision_id) > 0 AND created_at_unix >= 0
                ),
                CONSTRAINT fk_scope_request_revision_context_receipts_revision
                    FOREIGN KEY (request_id, revision_id)
                    REFERENCES scope_request_revisions(request_id, id) ON DELETE CASCADE
            );
        "#).await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Migration(
            "private request stacks are forward-only".into(),
        ))
    }
}
