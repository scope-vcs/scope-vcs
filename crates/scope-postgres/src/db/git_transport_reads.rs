use super::{
    RepositoryStore, begin_metadata_read_snapshot, entities,
    git_segments::load_git_pack_spans,
    history_rows::{RepositoryProjectionSource, load_repository_projection_sources},
    repository_access::load_repo_record,
};
use crate::error::PostgresError;
use scope_domain::repository::{
    RepositoryIncarnation,
    access::RepositoryAccessContext,
    git::{GitHead, GitPackSpan},
    repo_id,
};
use sea_orm::EntityTrait;

#[derive(Clone, Debug)]
pub struct GitReadSource {
    pub context: RepositoryAccessContext,
    pub public_files_visible: bool,
    pub git_head: Option<GitHead>,
    pub git_pack_spans: Vec<GitPackSpan>,
}

impl RepositoryStore {
    pub async fn git_read_source(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<GitReadSource>, PostgresError> {
        let Some((tx, context, public_files_visible)) = self
            .begin_access_snapshot(&repo_id(owner, name), viewer_user_id)
            .await?
        else {
            return Ok(None);
        };
        let git_head = entities::git_head::Entity::find_by_id(context.record.id.as_str())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map(entities::git_head::Model::try_into_domain)
            .transpose()?;
        let git_pack_spans = load_git_pack_spans(&tx, &context.record.id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(GitReadSource {
            context,
            public_files_visible,
            git_head,
            git_pack_spans,
        }))
    }

    pub async fn repository_projection_source(
        &self,
        incarnation: &RepositoryIncarnation,
        content_version: u64,
    ) -> Result<RepositoryProjectionSource, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let record = load_repo_record(&tx, incarnation.repository_id())
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        if record.incarnation() != *incarnation || record.content_version != content_version {
            return Err(PostgresError::conflict(
                "repository content changed while reading its history; retry",
            ));
        }
        let repo_ids = [record.id];
        let source = load_repository_projection_sources(&tx, &repo_ids)
            .await?
            .remove(&repo_ids[0])
            .ok_or_else(|| PostgresError::internal_message("repository history missing"))?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(source)
    }
}

#[cfg(test)]
mod tests;
