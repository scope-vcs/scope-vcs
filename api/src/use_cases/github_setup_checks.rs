use super::{github_check_results::refresh_commit_checks, github_workflow_runs};
use crate::{error::ApiError, repo_events::RepoChangeReason, state::AppState};
use scope_domain::github_setup_check::{GitHubSetupCheck, GitHubSetupCheckState};

const BATCH_SIZE: u64 = 20;

pub(crate) async fn reconcile_github_setup_checks_once(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let repositories = state.metadata.repositories();
    let mut ended = 0;
    for check in repositories.running_github_setup_checks(BATCH_SIZE).await? {
        if check.state == GitHubSetupCheckState::Waiting
            && let Err(error) = read_from_github(state, &check).await
        {
            tracing::warn!(
                repo_id = check.repository_id,
                error = %error.operator_diagnostic(),
                "reading a GitHub connection test failed; the next pass tries again"
            );
        }
        if repositories
            .observe_github_setup_check(&check.repository_id, &check.commit_oid, now_unix)
            .await?
            .is_some()
        {
            ended += 1;
            state.github_push_wakeup.notify_one();
            publish_setup_check_change(state, &check.repository_id).await?;
        }
    }
    Ok(ended)
}

async fn read_from_github(state: &AppState, check: &GitHubSetupCheck) -> Result<(), ApiError> {
    let Some(connection) = state
        .metadata
        .repositories()
        .github_connection(&check.repository_id)
        .await?
        .map(|read| read.connection)
        .filter(|connection| {
            connection.is_connected()
                && connection.github_repository_id == check.github_repository_id
        })
    else {
        return Ok(());
    };
    github_workflow_runs::refresh_branch_workflow_runs(
        state,
        &connection,
        &GitHubSetupCheck::branch(),
        &check.commit_oid,
    )
    .await?;
    refresh_commit_checks(state, &connection, &check.commit_oid).await
}

pub(crate) async fn publish_setup_check_change(
    state: &AppState,
    repo_id: &str,
) -> Result<(), ApiError> {
    if let Some(record) = state
        .metadata
        .repositories()
        .repository_record(repo_id)
        .await?
    {
        state
            .publish_request_summary_refresh(
                &record.incarnation(),
                RepoChangeReason::GitHubConnectionChanged,
            )
            .await;
    }
    Ok(())
}
