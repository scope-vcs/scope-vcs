use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0106_pool_bundle_backfill"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(
            "CREATE TABLE scope_pool_bundle_backfill_lease (
                singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
                token text,
                needs_scan boolean NOT NULL DEFAULT true,
                available_at_unix bigint NOT NULL DEFAULT 0 CHECK (available_at_unix >= 0)
            );
            INSERT INTO scope_pool_bundle_backfill_lease (singleton) VALUES (true);
            CREATE TABLE scope_pool_bundle_backfill_failures (
                source_id text PRIMARY KEY,
                request_id character varying NOT NULL REFERENCES scope_requests(id) ON DELETE CASCADE,
                reason text NOT NULL CHECK (length(reason) > 0),
                attempted_at_unix bigint NOT NULL CHECK (attempted_at_unix >= 0),
                retry_at_unix bigint NOT NULL CHECK (retry_at_unix >= attempted_at_unix)
            );
            CREATE INDEX scope_pool_bundle_backfill_retry ON scope_pool_bundle_backfill_failures (retry_at_unix);
            CREATE TABLE scope_pool_bundle_backfill_verifications (
                source_id text PRIMARY KEY,
                request_id character varying NOT NULL REFERENCES scope_requests(id) ON DELETE CASCADE,
                head_oid text NOT NULL,
                old_head_oid text NOT NULL,
                audience text NOT NULL,
                incarnation_id text NOT NULL,
                repo_config jsonb NOT NULL,
                pack_ids text[] NOT NULL,
                request_pack_ids text[] NOT NULL,
                reader_request_capacities jsonb NOT NULL,
                verified_at_unix bigint NOT NULL CHECK (verified_at_unix >= 0)
            );
            CREATE INDEX scope_pool_bundle_backfill_verifications_packs
                ON scope_pool_bundle_backfill_verifications USING gin (pack_ids);
            CREATE FUNCTION scope_wake_pool_bundle_backfill() RETURNS trigger LANGUAGE plpgsql AS $$
            BEGIN
                UPDATE scope_pool_bundle_backfill_lease SET needs_scan = true,
                    available_at_unix = CASE WHEN token IS NULL THEN 0 ELSE available_at_unix END
                    WHERE singleton;
                RETURN NULL;
            END;
            $$;
            CREATE TRIGGER scope_backfill_revision_change AFTER INSERT OR UPDATE OR DELETE
                ON scope_request_revisions FOR EACH STATEMENT EXECUTE FUNCTION scope_wake_pool_bundle_backfill();
            CREATE TRIGGER scope_backfill_request_change AFTER INSERT OR UPDATE OR DELETE
                ON scope_requests FOR EACH STATEMENT EXECUTE FUNCTION scope_wake_pool_bundle_backfill();
            CREATE FUNCTION scope_invalidate_pool_bundle_verification() RETURNS trigger LANGUAGE plpgsql AS $$
            BEGIN
                DELETE FROM scope_pool_bundle_backfill_verifications WHERE pack_ids @> ARRAY[OLD.id::text];
                RETURN NULL;
            END;
            $$;
            CREATE TRIGGER scope_backfill_pack_invalidation AFTER DELETE OR UPDATE
                ON scope_git_pool_packs FOR EACH ROW EXECUTE FUNCTION scope_invalidate_pool_bundle_verification();
            CREATE TRIGGER scope_backfill_pack_change AFTER INSERT OR DELETE OR UPDATE
                ON scope_git_pool_packs FOR EACH STATEMENT EXECUTE FUNCTION scope_wake_pool_bundle_backfill();
            CREATE TRIGGER scope_backfill_repository_change AFTER UPDATE OR DELETE
                ON scope_repositories FOR EACH STATEMENT EXECUTE FUNCTION scope_wake_pool_bundle_backfill();
            CREATE INDEX scope_git_pool_packs_backfill_head
                ON scope_git_pool_packs (request_id, head_oid);"
        ).await?;
        Ok(())
    }
}
