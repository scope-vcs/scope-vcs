use super::entities;
use crate::error::PostgresError;
use scope_domain::landing_file::{RepositoryLandingFile, RepositoryLandingFileMutation};
use sea_orm::{ConnectionTrait, EntityTrait, IntoActiveModel, sea_query::OnConflict};

pub(super) async fn apply_repository_landing_file_mutation<C>(
    conn: &C,
    repo_id: &str,
    mutation: RepositoryLandingFileMutation,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    match mutation {
        RepositoryLandingFileMutation::Unchanged => Ok(()),
        RepositoryLandingFileMutation::Upsert(landing_file) => {
            let model =
                entities::repository_landing_file::Model::from_domain(repo_id, landing_file)?;
            entities::repository_landing_file::Entity::insert(model.into_active_model())
                .on_conflict(
                    OnConflict::column(entities::repository_landing_file::Column::RepoId)
                        .update_columns([
                            entities::repository_landing_file::Column::Path,
                            entities::repository_landing_file::Column::Oid,
                            entities::repository_landing_file::Column::Sha256,
                            entities::repository_landing_file::Column::SizeBytes,
                            entities::repository_landing_file::Column::GitFileMode,
                            entities::repository_landing_file::Column::ContentBytes,
                        ])
                        .to_owned(),
                )
                .exec(conn)
                .await
                .map_err(PostgresError::internal)?;
            Ok(())
        }
        RepositoryLandingFileMutation::Delete => {
            entities::repository_landing_file::Entity::delete_by_id(repo_id.to_string())
                .exec(conn)
                .await
                .map_err(PostgresError::internal)?;
            Ok(())
        }
    }
}

pub(super) async fn repository_landing_file<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<RepositoryLandingFile>, PostgresError>
where
    C: ConnectionTrait,
{
    entities::repository_landing_file::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::repository_landing_file::Model::try_into_domain)
        .transpose()
}

#[cfg(test)]
mod tests;
