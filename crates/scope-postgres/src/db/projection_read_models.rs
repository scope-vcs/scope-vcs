use super::{entities, integer_columns::u64_to_i64};
use sea_orm::{ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
use {
    crate::error::PostgresError,
    scope_domain::{
        policy::ScopePath,
        projection::LabelledFiles,
        projection_views::{ProjectionViewFile, ProjectionViewFileContent},
        repo_config::RepoConfig,
        repo_control::{LEGACY_REPO_RULES_PATH, REPO_CONTROL_PREFIX, REPO_CONTROL_ROOT},
        views::{ViewId, Views},
    },
};

mod fold;

pub use fold::fold_live_projection_read_models;
pub(super) use fold::{build_projection_read_model, eager_views};

pub(super) enum ProjectionFileLookup {
    Found(ProjectionViewFileContent),
    Missing,
    NotReady,
}

pub(super) async fn repository_views<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Views, PostgresError> {
    let row = entities::repository::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("repository is missing"))?;
    let config: RepoConfig = super::decode_json(row.repo_config)?;
    Ok(config.views)
}

pub(super) async fn reset_live_projection_read_models<C>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::projection_read_model::Entity::delete_many()
        .filter(entities::projection_read_model::Column::RepoId.eq(repo_id.to_string()))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    super::history_reads::delete_history_payloads(conn, repo_id).await?;
    entities::projection_file::Entity::delete_many()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(())
}

fn view_condition(views: &Views, view: &ViewId) -> Condition {
    let labels = views
        .labels(view)
        .iter()
        .map(|label| label.as_str().to_string())
        .collect::<Vec<_>>();
    let mut condition =
        Condition::all().add(entities::projection_file::Column::Visibility.is_in(labels));
    if view != views.full() {
        condition = condition.add(
            Condition::any()
                .add(entities::projection_file::Column::Path.eq(LEGACY_REPO_RULES_PATH))
                .add(
                    Condition::all()
                        .add(entities::projection_file::Column::Path.ne(REPO_CONTROL_ROOT))
                        .add(
                            entities::projection_file::Column::Path
                                .not_like(format!("{REPO_CONTROL_PREFIX}%")),
                        ),
                ),
        );
    }
    condition
}

async fn load_projection_files<C>(conn: &C, repo_id: &str) -> Result<LabelledFiles, PostgresError>
where
    C: ConnectionTrait,
{
    entities::projection_file::Entity::find()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| {
            let content = row.try_into_content()?;
            Ok((content.file.path, (content.blob, content.file.label)))
        })
        .collect()
}

pub(super) async fn live_projection_read_model<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
    view: &ViewId,
) -> Result<Option<entities::projection_read_model::Model>, PostgresError>
where
    C: ConnectionTrait,
{
    let repo_version = u64_to_i64(repo_version, "repository version")?;
    Ok(entities::projection_read_model::Entity::find_by_id((
        repo_id.to_string(),
        view.as_str().to_string(),
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    .filter(|row| row.current() && row.repo_version == repo_version))
}

pub(super) async fn load_live_projection_file_for_view<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
    view: &ViewId,
    path: &ScopePath,
) -> Result<ProjectionFileLookup, PostgresError>
where
    C: ConnectionTrait,
{
    if live_projection_read_model(conn, repo_id, repo_version, view)
        .await?
        .is_none()
    {
        return Ok(ProjectionFileLookup::NotReady);
    }
    let row = entities::projection_file::Entity::find_by_id((
        repo_id.to_string(),
        entities::projection_file::projection_file_path_key(path),
    ))
    .filter(entities::projection_file::Column::Path.eq(path.as_str().to_string()))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?;
    match row {
        Some(row) => {
            let content = row.try_into_content()?;
            if repository_views(conn, repo_id).await?.shows(
                view,
                &content.file.path,
                &content.file.label,
            ) {
                Ok(ProjectionFileLookup::Found(content))
            } else {
                Ok(ProjectionFileLookup::Missing)
            }
        }
        None => Ok(ProjectionFileLookup::Missing),
    }
}

pub(super) async fn load_live_projection_files_for_view<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
    view: &ViewId,
) -> Result<Option<Vec<ProjectionViewFile>>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(model) = live_projection_read_model(conn, repo_id, repo_version, view).await? else {
        return Ok(None);
    };
    let views = repository_views(conn, repo_id).await?;
    let rows = entities::projection_file::Entity::find()
        .filter(entities::projection_file::Column::RepoId.eq(repo_id.to_string()))
        .filter(view_condition(&views, view))
        .order_by_asc(entities::projection_file::Column::Path)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let expected_file_count = usize::try_from(model.file_count)
        .map_err(|_| PostgresError::internal_message("projection file count cannot be negative"))?;
    if rows.len() != expected_file_count {
        return Ok(None);
    }

    let mut files = Vec::with_capacity(rows.len());
    for row in rows {
        let row_path = row.path.clone();
        match row.try_into_view() {
            Ok(file) => files.push(file),
            Err(error) => {
                tracing::warn!(
                    repo_id,
                    path = %row_path,
                    error = %error.message,
                    "ignoring invalid projection read-model row"
                );
                return Ok(None);
            }
        }
    }
    Ok(Some(files))
}

#[cfg(test)]
mod tests;
