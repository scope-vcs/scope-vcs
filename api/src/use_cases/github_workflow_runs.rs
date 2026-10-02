//! Reads the workflow runs GitHub Actions reports for connected repositories,
//! which their Runs pages list. A `workflow_run` delivery names the run to read
//! again; GitHub's API answers what it is now, so a late or repeated delivery
//! cannot leave an older state behind.

use crate::{error::ApiError, state::AppState};
use scope_domain::{github_connection::GitHubConnection, requests::GitHubBranch};

/// A delivery said a workflow run changed. Repositories that are not
/// connected are acknowledged and ignored.
pub(crate) async fn refresh_workflow_run_for_delivery(
    state: &AppState,
    github_repository_id: u64,
    run_id: u64,
) -> Result<(), ApiError> {
    let Some(connection) = state
        .metadata
        .repositories()
        .github_connection_for_github_repository(github_repository_id)
        .await?
    else {
        return Ok(());
    };
    let Some(run) = configured_app(state)?
        .workflow_run(
            connection.installation_id,
            &connection.github_full_name,
            run_id,
        )
        .await?
    else {
        return Ok(());
    };
    state
        .metadata
        .repositories()
        .save_github_workflow_run(
            &connection.repository_id,
            connection.github_repository_id,
            &run,
        )
        .await?;
    publish(state, &connection).await
}

/// Reads every workflow run GitHub started on one of Scope's branches for a
/// commit. A connection test reads them itself, so it does not depend on
/// deliveries arriving.
pub(crate) async fn refresh_branch_workflow_runs(
    state: &AppState,
    connection: &GitHubConnection,
    branch: &GitHubBranch,
    commit_oid: &str,
) -> Result<(), ApiError> {
    let Some(runs) = configured_app(state)?
        .branch_workflow_runs(
            connection.installation_id,
            &connection.github_full_name,
            &branch.name(),
            commit_oid,
        )
        .await?
    else {
        return Ok(());
    };
    let repositories = state.metadata.repositories();
    for run in &runs {
        repositories
            .save_github_workflow_run(
                &connection.repository_id,
                connection.github_repository_id,
                run,
            )
            .await?;
    }
    if !runs.is_empty() {
        publish(state, connection).await?;
    }
    Ok(())
}

async fn publish(state: &AppState, connection: &GitHubConnection) -> Result<(), ApiError> {
    if let Some(record) = state
        .metadata
        .repositories()
        .repository_record(&connection.repository_id)
        .await?
    {
        state
            .publish_github_workflow_runs_change(&record.incarnation())
            .await;
    }
    Ok(())
}

fn configured_app(state: &AppState) -> Result<&crate::github::GitHubApp, ApiError> {
    state
        .github
        .as_deref()
        .ok_or_else(|| ApiError::not_found("GitHub is not configured on this server"))
}
