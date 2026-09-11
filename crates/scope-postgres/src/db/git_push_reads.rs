use super::{
    RepositoryStore, begin_metadata_read_snapshot, entities, git_segments::load_git_pack_spans,
};
use sea_orm::{ConnectionTrait, EntityTrait};
use {
    crate::error::PostgresError,
    scope_domain::{
        repo_config::RepoConfig,
        repository::access::{RepositoryAccess, repository_access_for_user_id},
        repository::git::{GitHead, GitPackSpan},
        repository::{RepoLifecycleState, RepositoryIncarnation, repo_id},
    },
};

#[derive(Clone, Debug)]
pub struct GitPushContext {
    pub repo_id: String,
    pub incarnation: RepositoryIncarnation,
    pub owner_user_id: String,
    pub lifecycle_state: RepoLifecycleState,
    pub access: RepositoryAccess,
    pub repo_config: RepoConfig,
    pub git_head: Option<GitHead>,
    pub git_pack_spans: Vec<GitPackSpan>,
    pub change_version: u64,
}

impl RepositoryStore {
    pub async fn run_repository_incarnation(
        &self,
        run_id: &str,
        expected_repository_id: &str,
    ) -> Result<Option<RepositoryIncarnation>, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(run) = entities::run::Entity::find_by_id(run_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        if run.repo_id != expected_repository_id {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        }
        let repository = entities::repository::Entity::find_by_id(&run.repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        repository
            .map(|row| RepositoryIncarnation::new(row.id, row.incarnation_id))
            .transpose()
            .map_err(PostgresError::internal)
    }

    pub async fn git_push_context(
        &self,
        owner: &str,
        name: &str,
        user_id: &str,
    ) -> Result<Option<GitPushContext>, PostgresError> {
        let id = repo_id(owner, name);
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let context = git_push_context_for_id(&tx, &id, user_id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(context)
    }
}

pub(super) async fn git_push_context_for_id<C: ConnectionTrait>(
    conn: &C,
    id: &str,
    user_id: &str,
) -> Result<Option<GitPushContext>, PostgresError> {
    let id = id.to_string();
    let Some(repo_row) = entities::repository::Entity::find_by_id(id.clone())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    let head = entities::git_head::Entity::find_by_id(id.clone())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::git_head::Model::try_into_domain)
        .transpose()?;
    let pack_spans = load_git_pack_spans(conn, &id).await?;
    let permissions =
        entities::repository_member::Entity::find_by_id((id.clone(), user_id.to_string()))
            .one(conn)
            .await
            .map_err(PostgresError::internal)?
            .map(entities::repository_member::Model::try_into_domain)
            .transpose()?
            .map(|member| member.permissions);
    let lifecycle_state = entities::decode_enum(repo_row.publication_state)?;
    let access = repository_access_for_user_id(
        &repo_row.owner_user_id,
        lifecycle_state,
        permissions,
        user_id,
    );
    let context = GitPushContext {
        incarnation: RepositoryIncarnation::new(id.clone(), repo_row.incarnation_id)
            .map_err(PostgresError::internal)?,
        repo_id: id,
        owner_user_id: repo_row.owner_user_id,
        lifecycle_state,
        access,
        repo_config: serde_json::from_value(repo_row.repo_config)
            .map_err(PostgresError::internal)?,
        git_head: head,
        git_pack_spans: pack_spans,
        change_version: entities::i64_to_u64(repo_row.change_version, "repository change version")?,
    };
    Ok(Some(context))
}
