use super::{RepositoryStore, begin_metadata_read_snapshot, entities};
use crate::error::PostgresError;
use scope_domain::{
    repository::git::GitHead,
    runs::catalog::{RepositoryWorkflowCatalog, RepositoryWorkflowFile},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, sea_query::OnConflict,
};

pub struct CurrentRepositoryWorkflowCatalog {
    pub repository_id: String,
    pub git_head: Option<GitHead>,
    pub catalog: Option<RepositoryWorkflowCatalog>,
}

#[cfg(test)]
mod tests;

pub(super) async fn apply_repository_workflow_catalog<C>(
    conn: &C,
    catalog: &RepositoryWorkflowCatalog,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    catalog
        .validate_integrity()
        .map_err(PostgresError::internal)?;
    let header = catalog_header_from_domain(catalog)?;
    let repo_id = header.repo_id.clone();

    entities::repository_workflow_file::Entity::delete_many()
        .filter(entities::repository_workflow_file::Column::RepoId.eq(&repo_id))
        .exec(conn)
        .await?;
    entities::repository_workflow_catalog::Entity::insert(header.into_active_model())
        .on_conflict(
            OnConflict::column(entities::repository_workflow_catalog::Column::RepoId)
                .update_columns([
                    entities::repository_workflow_catalog::Column::SourceHeadOid,
                    entities::repository_workflow_catalog::Column::SourceChangeVersion,
                    entities::repository_workflow_catalog::Column::ConfigurationError,
                ])
                .to_owned(),
        )
        .exec(conn)
        .await?;

    for file in catalog.files().unwrap_or_default() {
        workflow_file_from_domain(&repo_id, file)?
            .into_active_model()
            .insert(conn)
            .await?;
    }
    Ok(())
}

pub(super) async fn repository_workflow_catalog<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<RepositoryWorkflowCatalog>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(header) = entities::repository_workflow_catalog::Entity::find_by_id(repo_id)
        .one(conn)
        .await?
    else {
        return Ok(None);
    };
    catalog_from_rows(conn, header).await.map(Some)
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_workflow_catalogs"))]
    pub async fn repository_workflow_catalogs(
        &self,
    ) -> Result<Vec<RepositoryWorkflowCatalog>, PostgresError> {
        load_repository_workflow_catalogs(self.db.as_ref()).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "current_repository_workflow_catalog"))]
    pub async fn current_repository_workflow_catalog(
        &self,
        repo_id: &str,
    ) -> Result<Option<CurrentRepositoryWorkflowCatalog>, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let snapshot = match entities::repository::Entity::find_by_id(repo_id)
            .select_only()
            .column(entities::repository::Column::Id)
            .into_tuple::<String>()
            .one(&tx)
            .await?
        {
            Some(repository_id) => Some(CurrentRepositoryWorkflowCatalog {
                repository_id,
                git_head: entities::git_head::Entity::find_by_id(repo_id)
                    .one(&tx)
                    .await?
                    .map(entities::git_head::Model::try_into_domain)
                    .transpose()?,
                catalog: repository_workflow_catalog(&tx, repo_id).await?,
            }),
            None => None,
        };
        tx.commit().await?;
        Ok(snapshot)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn delete_repository_workflow_catalog_for_tests(
        &self,
        repo_id: &str,
    ) -> Result<(), PostgresError> {
        entities::repository_workflow_catalog::Entity::delete_by_id(repo_id)
            .exec(self.db.as_ref())
            .await?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn set_repository_workflow_catalog_source_for_tests(
        &self,
        repo_id: &str,
        source_head_oid: &str,
    ) -> Result<(), PostgresError> {
        let statement = sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE scope_repository_workflow_catalogs
             SET source_head_oid = $2
             WHERE repo_id = $1",
            [repo_id.into(), source_head_oid.into()],
        );
        self.db.execute_raw(statement).await?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn corrupt_repository_workflow_file_content_for_tests(
        &self,
        repo_id: &str,
        path: &str,
        content_bytes: Vec<u8>,
    ) -> Result<(), PostgresError> {
        let statement = sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE scope_repository_workflow_files
             SET content_bytes = $3,
                 size_bytes = octet_length($3::bytea)
             WHERE repo_id = $1 AND path = $2",
            [repo_id.into(), path.into(), content_bytes.into()],
        );
        self.db.execute_raw(statement).await?;
        Ok(())
    }
}

pub(super) async fn load_repository_workflow_catalogs(
    db: &DatabaseConnection,
) -> Result<Vec<RepositoryWorkflowCatalog>, PostgresError> {
    let tx = begin_metadata_read_snapshot(db).await?;
    let headers = entities::repository_workflow_catalog::Entity::find()
        .order_by_asc(entities::repository_workflow_catalog::Column::RepoId)
        .all(&tx)
        .await?;
    let mut catalogs = Vec::with_capacity(headers.len());
    for header in headers {
        catalogs.push(catalog_from_rows(&tx, header).await?);
    }
    tx.commit().await?;
    Ok(catalogs)
}

fn catalog_header_from_domain(
    catalog: &RepositoryWorkflowCatalog,
) -> Result<entities::repository_workflow_catalog::Model, PostgresError> {
    Ok(entities::repository_workflow_catalog::Model {
        repo_id: catalog.repository_id().to_string(),
        source_head_oid: catalog.source_head_oid().to_string(),
        source_change_version: i64::try_from(catalog.source_change_version()).map_err(|_| {
            PostgresError::internal_message(
                "repository workflow catalog change version exceeds PostgreSQL bigint range",
            )
        })?,
        configuration_error: catalog.configuration_error().map(str::to_string),
    })
}

fn workflow_file_from_domain(
    repo_id: &str,
    file: &RepositoryWorkflowFile,
) -> Result<entities::repository_workflow_file::Model, PostgresError> {
    file.validate_integrity().map_err(PostgresError::internal)?;
    Ok(entities::repository_workflow_file::Model {
        repo_id: repo_id.to_string(),
        path: file.path().as_str().to_string(),
        oid: file.oid().to_string(),
        size_bytes: i64::try_from(file.size_bytes()).map_err(|_| {
            PostgresError::internal_message(
                "repository workflow file size exceeds PostgreSQL bigint range",
            )
        })?,
        git_file_mode: file.git_file_mode().to_string(),
        content_bytes: file.content_bytes().to_vec(),
    })
}

async fn catalog_from_rows(
    conn: &impl ConnectionTrait,
    header: entities::repository_workflow_catalog::Model,
) -> Result<RepositoryWorkflowCatalog, PostgresError> {
    let files = entities::repository_workflow_file::Entity::find()
        .filter(entities::repository_workflow_file::Column::RepoId.eq(header.repo_id.clone()))
        .order_by_asc(entities::repository_workflow_file::Column::Path)
        .all(conn)
        .await?;
    let source_change_version = u64::try_from(header.source_change_version).map_err(|_| {
        PostgresError::internal_message(
            "repository workflow catalog change version cannot be negative",
        )
    })?;
    match header.configuration_error {
        Some(error) if files.is_empty() => RepositoryWorkflowCatalog::rejected(
            header.repo_id,
            header.source_head_oid,
            source_change_version,
            error,
        )
        .map_err(PostgresError::internal),
        Some(_) => Err(PostgresError::internal_message(
            "rejected repository workflow catalog contains files",
        )),
        None => RepositoryWorkflowCatalog::captured(
            header.repo_id,
            header.source_head_oid,
            source_change_version,
            files
                .into_iter()
                .map(workflow_file_from_row)
                .collect::<Result<Vec<_>, _>>()?,
        )
        .map_err(PostgresError::internal),
    }
}

fn workflow_file_from_row(
    row: entities::repository_workflow_file::Model,
) -> Result<RepositoryWorkflowFile, PostgresError> {
    RepositoryWorkflowFile::new(
        row.path,
        row.oid,
        u64::try_from(row.size_bytes).map_err(|_| {
            PostgresError::internal_message("repository workflow file size cannot be negative")
        })?,
        row.git_file_mode,
        row.content_bytes,
    )
    .map_err(PostgresError::internal)
}
