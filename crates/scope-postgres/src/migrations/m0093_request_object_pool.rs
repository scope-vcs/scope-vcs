use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0093_request_object_pool"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_git_pool_packs (
                    id text PRIMARY KEY,
                    repo_id character varying NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    request_id character varying NOT NULL
                        REFERENCES scope_requests(id) ON DELETE CASCADE,
                    head_oid text NOT NULL,
                    prerequisites text[] NOT NULL,
                    sealed_pack bytea,
                    segment_id text UNIQUE,
                    segment_sha256 text,
                    segment_plaintext_bytes bigint,
                    segment_encoding_version integer,
                    created_at_unix bigint NOT NULL,
                    CONSTRAINT scope_git_pool_pack_values CHECK (
                        length(btrim(id)) > 0
                        AND length(btrim(head_oid)) > 0
                        AND created_at_unix >= 0
                    ),
                    CONSTRAINT scope_git_pool_pack_storage CHECK (
                        (sealed_pack IS NOT NULL AND segment_id IS NULL
                            AND segment_sha256 IS NULL AND segment_plaintext_bytes IS NULL
                            AND segment_encoding_version IS NULL)
                        OR (sealed_pack IS NULL AND segment_id IS NOT NULL
                            AND length(segment_sha256) = 64
                            AND segment_plaintext_bytes >= 0
                            AND segment_encoding_version > 0)
                    )
                );
                CREATE INDEX scope_git_pool_packs_request
                    ON scope_git_pool_packs (request_id, created_at_unix, id);
                DROP TABLE scope_request_ref_cleanup_jobs;
                "#,
            )
            .await?;
        Ok(())
    }
}
