use super::{
    RepositoryStore, begin_metadata_read_snapshot, entities,
    git_segments::load_git_pack_spans,
    history_rows::{RepositoryProjectionSource, load_repository_projection_sources},
    projection_read_models::{live_projection_read_model, repository_views},
    repository_access::load_repo_record,
};
use crate::error::PostgresError;
use scope_domain::{
    policy::ScopePath,
    repository::{
        RepoRecord, RepositoryIncarnation,
        access::RepositoryAccessContext,
        git::{GitHead, GitPackSpan},
        repo_id,
    },
    requests::PathHistory,
    views::{ViewId, Views},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};

#[derive(Clone, Debug)]
pub struct GitReadSource {
    pub context: RepositoryAccessContext,
    pub public_files_visible: bool,
    pub git_head: Option<GitHead>,
    pub git_pack_spans: Vec<GitPackSpan>,
}

#[derive(Clone, Debug)]
pub struct RepositoryGitState {
    pub content_version: u64,
    pub git_head: Option<GitHead>,
    pub git_pack_spans: Vec<GitPackSpan>,
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_git_state"))]
    pub async fn repository_git_state(
        &self,
        incarnation: &RepositoryIncarnation,
    ) -> Result<RepositoryGitState, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let record = load_repo_record(&tx, incarnation.repository_id())
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        if record.incarnation() != *incarnation {
            return Err(PostgresError::conflict("repository was recreated; retry"));
        }
        let git_head = entities::git_head::Entity::find_by_id(incarnation.repository_id())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map(entities::git_head::Model::try_into_domain)
            .transpose()?;
        let git_pack_spans = load_git_pack_spans(&tx, incarnation.repository_id()).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RepositoryGitState {
            content_version: record.content_version,
            git_head,
            git_pack_spans,
        })
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "git_read_source"))]
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

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_projection_source"))]
    pub async fn repository_projection_source(
        &self,
        incarnation: &RepositoryIncarnation,
        content_version: u64,
    ) -> Result<RepositoryProjectionSource, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let record = load_record_at_version(&tx, incarnation, content_version).await?;
        let repo_ids = [record.id];
        let source = load_repository_projection_sources(&tx, &repo_ids)
            .await?
            .remove(&repo_ids[0])
            .ok_or_else(|| PostgresError::internal_message("repository history missing"))?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(source)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_views"))]
    pub async fn repository_views(
        &self,
        incarnation: &RepositoryIncarnation,
        content_version: u64,
    ) -> Result<Views, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let record = load_record_at_version(&tx, incarnation, content_version).await?;
        let views = repository_views(&tx, &record.id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(views)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_view_head"))]
    pub async fn repository_view_head(
        &self,
        incarnation: &RepositoryIncarnation,
        content_version: u64,
        view: &ViewId,
    ) -> Result<Option<String>, PostgresError> {
        for _ in 0..2 {
            let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
            let record = load_record_at_version(&tx, incarnation, content_version).await?;
            let read_model =
                live_projection_read_model(&tx, &record.id, content_version, view).await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            if let Some(read_model) = read_model {
                return Ok(read_model.head_oid);
            }
            self.ensure_live_projection_read_models(incarnation, view)
                .await?;
        }
        Err(PostgresError::conflict(
            "repository history view kept changing; retry",
        ))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_path_history"))]
    pub async fn repository_path_history(
        &self,
        incarnation: &RepositoryIncarnation,
        content_version: u64,
        paths: &[ScopePath],
    ) -> Result<PathHistory, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let record = load_record_at_version(&tx, incarnation, content_version).await?;
        let mut history = PathHistory::default();
        for paths in paths.chunks(PATH_BATCH_SIZE) {
            let paths = paths.iter().map(ScopePath::as_str).collect::<Vec<_>>();
            use entities::{file_change, live_file, visibility_change};
            for path in live_file::Entity::find()
                .select_only()
                .column(live_file::Column::Path)
                .filter(live_file::Column::RepoId.eq(record.id.as_str()))
                .filter(live_file::Column::Path.is_in(paths.clone()))
                .into_tuple::<String>()
                .all(&tx)
                .await
                .map_err(PostgresError::internal)?
            {
                history.live_paths.insert(scope_path(path)?);
            }
            for (path, label) in file_change::Entity::find()
                .select_only()
                .columns([file_change::Column::Path, file_change::Column::Visibility])
                .filter(file_change::Column::RepoId.eq(record.id.as_str()))
                .filter(file_change::Column::Path.is_in(paths.clone()))
                .into_tuple::<(String, String)>()
                .all(&tx)
                .await
                .map_err(PostgresError::internal)?
            {
                history
                    .file_change_labels
                    .push((scope_path(path)?, view_label(&label)?));
            }
            for (path, old_label, new_label) in visibility_change::Entity::find()
                .select_only()
                .columns([
                    visibility_change::Column::Path,
                    visibility_change::Column::OldVisibility,
                    visibility_change::Column::NewVisibility,
                ])
                .filter(visibility_change::Column::RepoId.eq(record.id.as_str()))
                .filter(visibility_change::Column::Path.is_in(paths))
                .into_tuple::<(String, String, String)>()
                .all(&tx)
                .await
                .map_err(PostgresError::internal)?
            {
                history.visibility_changes.push((
                    scope_path(path)?,
                    view_label(&old_label)?,
                    view_label(&new_label)?,
                ));
            }
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(history)
    }
}

const PATH_BATCH_SIZE: usize = 1000;

fn view_label(value: &str) -> Result<ViewId, PostgresError> {
    ViewId::parse(value).map_err(PostgresError::internal)
}

async fn load_record_at_version<C: ConnectionTrait>(
    conn: &C,
    incarnation: &RepositoryIncarnation,
    content_version: u64,
) -> Result<RepoRecord, PostgresError> {
    let record = load_repo_record(conn, incarnation.repository_id())
        .await?
        .ok_or_else(|| PostgresError::not_found("repo not found"))?;
    if record.incarnation() != *incarnation || record.content_version != content_version {
        return Err(PostgresError::conflict(
            "repository content changed while reading its history; retry",
        ));
    }
    Ok(record)
}

fn scope_path(path: String) -> Result<ScopePath, PostgresError> {
    ScopePath::parse(path).map_err(PostgresError::internal)
}

#[cfg(test)]
mod tests;
