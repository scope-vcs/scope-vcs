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
                    segment_id text PRIMARY KEY,
                    repo_id character varying NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    request_id character varying NOT NULL
                        REFERENCES scope_requests(id) ON DELETE CASCADE,
                    head_oid text NOT NULL,
                    prerequisites text[] NOT NULL,
                    sha256 text NOT NULL,
                    plaintext_bytes bigint NOT NULL,
                    encoding_version integer NOT NULL,
                    transaction_id bigint NOT NULL
                        DEFAULT (pg_current_xact_id()::text)::bigint,
                    created_at_unix bigint NOT NULL,
                    CONSTRAINT scope_git_pool_pack_values CHECK (
                        length(btrim(segment_id)) > 0
                        AND length(btrim(head_oid)) > 0
                        AND length(sha256) = 64
                        AND plaintext_bytes >= 0
                        AND encoding_version > 0
                        AND created_at_unix >= 0
                    )
                );
                CREATE INDEX scope_git_pool_packs_repo_transaction
                    ON scope_git_pool_packs (repo_id, transaction_id);
                DROP TABLE scope_request_ref_cleanup_jobs;
                "#,
            )
            .await?;
        Ok(())
    }
}
