use super::{
    RepositoryStore, entities, git_segments::register_git_pool_pack,
    locks::acquire_shared_repository_lock,
};
use crate::error::PostgresError;
use scope_domain::repository::{Repository, git::GitPoolPack};
use sea_orm::{DatabaseBackend, EntityTrait, FromQueryResult, Statement, TransactionTrait};

#[derive(Clone, FromQueryResult)]
pub struct BundleSource {
    pub source_id: String,
    pub stack_depth: i32,
    pub position: i64,
    pub repo_id: String,
    pub request_id: String,
    pub head_oid: String,
    pub old_head_oid: String,
    pub git_snapshot: sea_orm::JsonValue,
}

impl RepositoryStore {
    pub async fn bundle_batch(
        &self,
        after: Option<&str>,
        limit: Option<i64>,
    ) -> Result<Vec<BundleSource>, PostgresError> {
        let tx = super::connection::begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let sources = BundleSource::find_by_statement(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "WITH sources AS (
                SELECT v.id AS source_id, r.repo_id, r.id AS request_id, v.new_head_oid AS head_oid, v.old_head_oid,
                    v.git_snapshot, r.stack_depth, v.position
                FROM scope_request_revisions v JOIN scope_requests r ON r.id = v.request_id
                UNION ALL
                SELECT r.id, r.repo_id, r.id, r.head_oid, r.review_base_oid, r.git_snapshot,
                    r.stack_depth, 9223372036854775807
                FROM scope_requests r WHERE r.git_snapshot IS NOT NULL
                    AND NOT EXISTS (SELECT 1 FROM scope_request_revisions v
                        WHERE v.request_id = r.id AND v.new_head_oid = r.head_oid)
            ) SELECT * FROM sources WHERE ($1::text IS NULL OR source_id > $1) ORDER BY source_id LIMIT $2",
            [after.map(str::to_owned).into(), limit.into()],
        )).all(&tx).await?;
        tx.commit().await?;
        Ok(sources)
    }

    pub async fn retain_bundle_pool_pack(
        &self,
        expected: &Repository,
        pack: &GitPoolPack,
        now: u64,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await?;
        acquire_shared_repository_lock(&tx, &expected.record.id).await?;
        let repo = entities::repository::Entity::find_by_id(&expected.record.id)
            .one(&tx)
            .await?
            .ok_or_else(|| PostgresError::not_found("backfill repository missing"))?;
        if repo.incarnation_id != expected.record.incarnation_id
            || repo.repo_config
                != serde_json::to_value(&expected.repo_config).map_err(PostgresError::internal)?
        {
            return Err(PostgresError::conflict("backfill repository changed"));
        }
        register_git_pool_pack(&tx, &expected.record.id, pack, now).await?;
        tx.commit().await.map_err(Into::into)
    }
}
