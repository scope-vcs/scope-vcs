//! The accepted main workflow catalog and its source identity.

use crate::{error::ApiError, state::AppState};
use scope_domain::runs::{
    catalog::RepositoryWorkflowCatalog, workflow::revision::WorkflowRevision,
};

pub(crate) async fn current_catalog(
    state: &AppState,
    repository_id: &str,
) -> Result<Option<RepositoryWorkflowCatalog>, ApiError> {
    let snapshot = state
        .metadata
        .repositories()
        .current_repository_workflow_catalog(repository_id)
        .await?
        .ok_or_else(|| {
            ApiError::internal_message("repository disappeared while loading current workflows")
        })?;
    let Some(head) = snapshot.git_head.as_ref() else {
        return if snapshot.catalog.is_none() {
            Ok(None)
        } else {
            Err(ApiError::internal_message(
                "repository workflow catalog exists without an accepted Git head",
            ))
        };
    };
    let catalog = snapshot.catalog.ok_or_else(|| {
        ApiError::internal_message("repository workflow catalog is missing for current main")
    })?;
    catalog
        .verify_source(&snapshot.repository_id, &head.head_oid, head.change_version)
        .map_err(ApiError::internal)?;
    Ok(Some(catalog))
}

pub(crate) async fn current_workflows(
    state: &AppState,
    repository_id: &str,
) -> Result<Vec<WorkflowRevision>, ApiError> {
    let Some(catalog) = current_catalog(state, repository_id).await? else {
        return Ok(Vec::new());
    };
    scope_run_config::parse_repository_workflow_catalog(&catalog).map_err(ApiError::bad_request)
}
