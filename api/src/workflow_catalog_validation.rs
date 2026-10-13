use scope_domain::runs::catalog::RepositoryWorkflowCatalog;

pub async fn validate_repository_workflow_catalogs_for_maintenance(
    database_url: String,
) -> anyhow::Result<usize> {
    let catalogs =
        scope_postgres::db::repository_workflow_catalogs_for_maintenance(database_url).await?;
    validate_repository_workflow_catalogs(catalogs)
}

fn validate_repository_workflow_catalogs(
    catalogs: Vec<RepositoryWorkflowCatalog>,
) -> anyhow::Result<usize> {
    let mut validated = 0;
    for catalog in catalogs {
        if catalog.configuration_error().is_some() {
            continue;
        }
        scope_run_config::parse_repository_workflow_catalog(&catalog).map_err(|error| {
            anyhow::anyhow!(
                "repository {} workflow catalog at {} is invalid under this release: {error}",
                catalog.repository_id(),
                catalog.source_head_oid(),
            )
        })?;
        validated += 1;
    }
    Ok(validated)
}

#[cfg(test)]
impl crate::state::AppState {
    pub(crate) async fn validate_repository_workflow_catalogs(&self) -> anyhow::Result<usize> {
        let catalogs = self
            .metadata
            .repositories()
            .repository_workflow_catalogs()
            .await?;
        validate_repository_workflow_catalogs(catalogs)
    }
}
