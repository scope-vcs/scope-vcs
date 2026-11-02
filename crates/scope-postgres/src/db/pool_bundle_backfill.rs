use super::{
    RepositoryStore, entities, git_segments::register_git_pool_pack,
    locks::acquire_shared_repository_lock,
};
use crate::error::PostgresError;
use scope_domain::repository::{RepositoryIncarnation, git::GitPoolPack};
use sea_orm::{DatabaseBackend, EntityTrait, FromQueryResult, Statement, TransactionTrait};

#[derive(FromQueryResult)]
pub struct BundleSource {
    pub repo_id: String,
    pub request_id: String,
    pub view: String,
    pub head_oid: String,
    pub old_head_oid: String,
    pub git_snapshot: sea_orm::JsonValue,
}

impl RepositoryStore {
    pub async fn bundle_batch(&self, offset: u64) -> Result<Vec<BundleSource>, PostgresError> {
        BundleSource::find_by_statement(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "WITH sources AS (
                SELECT r.repo_id, r.id AS request_id, v.new_head_oid AS head_oid, v.old_head_oid,
                    v.git_snapshot, v.position, r.stack_depth, r.audience AS view
                FROM scope_request_revisions v JOIN scope_requests r ON r.id = v.request_id
                UNION ALL
                SELECT r.repo_id, r.id, r.head_oid, r.review_base_oid, r.git_snapshot,
                    9223372036854775807, r.stack_depth, r.audience
                FROM scope_requests r WHERE r.git_snapshot IS NOT NULL
                    AND NOT EXISTS (SELECT 1 FROM scope_request_revisions v
                        WHERE v.request_id = r.id AND v.new_head_oid = r.head_oid)
            ) SELECT * FROM sources ORDER BY repo_id, stack_depth, request_id, position LIMIT 32 OFFSET $1",
            [super::integer_columns::u64_to_i64(offset, "bundle revision offset")?.into()],
        )).all(self.db.as_ref()).await.map_err(Into::into)
    }

    pub async fn retain_bundle_pool_pack(
        &self,
        incarnation: &RepositoryIncarnation,
        generation: &str,
        pack: &GitPoolPack,
        now: u64,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await?;
        acquire_shared_repository_lock(&tx, incarnation.repository_id()).await?;
        let repo = entities::repository::Entity::find_by_id(incarnation.repository_id())
            .one(&tx)
            .await?
            .ok_or_else(|| PostgresError::not_found("backfill repository missing"))?;
        if repo.incarnation_id != incarnation.incarnation_id() {
            return Err(PostgresError::conflict("backfill repository changed"));
        }
        super::requests::ensure_object_generation(&tx, incarnation.repository_id(), generation)
            .await?;
        register_git_pool_pack(&tx, incarnation.repository_id(), pack, now).await?;
        tx.commit().await?;
        Ok(())
    }
}
