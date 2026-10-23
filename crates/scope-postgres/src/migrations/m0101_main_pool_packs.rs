use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0101_main_pool_packs"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_git_pool_packs
                    ALTER COLUMN request_id DROP NOT NULL,
                    ADD COLUMN first_sequence bigint,
                    ADD COLUMN last_sequence bigint,
                    ADD CONSTRAINT scope_git_pool_pack_owner CHECK (
                        (request_id IS NULL) = (first_sequence IS NOT NULL)
                        AND (first_sequence IS NULL) = (last_sequence IS NULL)
                        AND (
                            first_sequence IS NULL
                            OR (first_sequence > 0 AND first_sequence <= last_sequence)
                        )
                    );

                CREATE UNIQUE INDEX scope_git_pool_packs_main_sequence
                    ON scope_git_pool_packs (repo_id, first_sequence)
                    WHERE request_id IS NULL;

                INSERT INTO scope_git_pool_packs (
                    id, repo_id, request_id, head_oid, prerequisites, sealed_pack, segment_id,
                    segment_sha256, segment_plaintext_bytes, segment_encoding_version,
                    created_at_unix, first_sequence, last_sequence
                )
                SELECT spans.segment_id, spans.repo_id, NULL, spans.head_oid,
                       CASE
                           WHEN spans.base_oid IS NULL THEN ARRAY[]::text[]
                           ELSE ARRAY[spans.base_oid::text]
                       END,
                       NULL, spans.segment_id, uploads.sha256, uploads.plaintext_bytes,
                       uploads.encoding_version, uploads.created_at_unix,
                       spans.first_sequence, spans.last_sequence
                FROM scope_git_segments spans
                JOIN scope_git_segment_uploads uploads ON uploads.segment_id = spans.segment_id;

                DROP TABLE scope_git_segments;
                "#,
            )
            .await?;
        Ok(())
    }
}
