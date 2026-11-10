use super::{
    RepositoryStore, RequestStore, acquire_aggregate_lock, entities,
    git_segments::register_git_pool_pack, repository_rows::repository_from_model,
    request_revision_rows::revision_by_id, request_rows::request_by_id,
};
use crate::error::PostgresError;
use scope_domain::{
    repository::{Repository, git::GitPoolPack},
    requests::{Request, RequestRevision},
};
use sea_orm::{DatabaseBackend, EntityTrait, FromQueryResult, Statement, TransactionTrait};

impl RequestStore {
    pub async fn revision_for_pool_recovery(
        &self,
        revision_id: &str,
    ) -> Result<Option<RequestRevision>, PostgresError> {
        revision_by_id(self.db.as_ref(), revision_id).await
    }

    pub async fn revision_pool_recovery_batch(
        &self,
        repo_id: Option<&str>,
        after: Option<&str>,
    ) -> Result<Vec<RequestRevision>, PostgresError> {
        entities::request_revision::Model::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT revisions.* FROM scope_request_revisions revisions
             JOIN scope_requests requests ON requests.id = revisions.request_id
             WHERE ($1::text IS NULL OR requests.repo_id = $1)
               AND ($2::text IS NULL OR revisions.id > $2)
             ORDER BY revisions.id LIMIT 100",
            [
                repo_id.map(str::to_string).into(),
                after.map(str::to_string).into(),
            ],
        ))
        .all(self.db.as_ref())
        .await?
        .into_iter()
        .map(entities::request_revision::Model::try_into_domain)
        .collect()
    }
}

impl RepositoryStore {
    pub async fn restore_revision_pool_pack(
        &self,
        observed_repo: &Repository,
        observed_request: &Request,
        observed_revision: &RequestRevision,
        pack: &GitPoolPack,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let repo_id = &observed_repo.record.id;
        let tx = self.db.begin().await?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await?
            .ok_or_else(|| PostgresError::not_found("restoration repository missing"))?;
        let repo = repository_from_model(&tx, row).await?;
        let request = request_by_id(&tx, &observed_request.id)
            .await?
            .ok_or_else(|| PostgresError::not_found("restoration request missing"))?;
        let revision = revision_by_id(&tx, &observed_revision.id)
            .await?
            .ok_or_else(|| PostgresError::not_found("restoration revision missing"))?;
        if repo.incarnation() != observed_repo.incarnation()
            || repo.repo_config.object_generation() != observed_repo.repo_config.object_generation()
            || request != *observed_request
            || revision != *observed_revision
            || request.repo_id != *repo_id
        {
            return Err(PostgresError::conflict(
                "restoration state changed; rerun dry run",
            ));
        }
        let owner = revision.restoration_pack_owner(&request, repo.repo_config.views())?;
        if pack.owner != owner
            || pack.head_oid != revision.new_head_oid
            || !pack.prerequisites.is_empty()
        {
            return Err(PostgresError::conflict(
                "restoration pack must be self-contained and match the revision",
            ));
        }
        use sea_orm::ConnectionTrait;
        let existing = tx
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT id FROM scope_git_pool_packs WHERE repo_id = $1 AND id = $2",
                [repo_id.clone().into(), pack.id.clone().into()],
            ))
            .await?;
        if existing.is_some() {
            tx.commit().await?;
            return Ok(false);
        }
        register_git_pool_pack(&tx, repo_id, pack, now_unix).await?;
        tx.commit().await?;
        Ok(true)
    }
}
