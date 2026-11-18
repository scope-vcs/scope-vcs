use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{
    after_commit::AfterCommit, error::ApiError, persistence::unix_now,
    repo_events::RepoChangeReason, state::AppState,
    use_cases::repository_collaboration::publish_committed_mutation,
};
use scope_postgres::db::{GitHubWorkflowRetentionCursor, GitHubWorkflowRetentionPage};
use std::{collections::BTreeSet, time::Duration};
use tokio::time::MissedTickBehavior;

const RETENTION_BATCH_SIZE: u64 = 100;
const RETENTION_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub(crate) async fn apply_invite_retention(
    state: &AppState,
    now_unix: u64,
) -> anyhow::Result<usize> {
    let repositories = state.metadata.repositories();
    let repo_ids = repositories
        .repositories_with_prunable_invites(now_unix, RETENTION_BATCH_SIZE)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let mut pruned = 0;
    for repo_id in repo_ids {
        let mutation = repositories
            .prune_ended_repository_invites(&repo_id, now_unix)
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
        if let Some(mutation) = mutation {
            pruned +=
                publish_committed_mutation(state, mutation, RepoChangeReason::InviteUpdated).await;
        }
    }
    Ok(pruned)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct GitHubWorkflowRetentionPass {
    deleted_runs: usize,
    expired_runs: usize,
    deleted_jobs_without_run: usize,
}

pub(crate) async fn apply_github_workflow_retention_page(
    state: &AppState,
    now_unix: u64,
    after: Option<GitHubWorkflowRetentionCursor>,
    page_size: u64,
) -> Result<GitHubWorkflowRetentionPage, ApiError> {
    let page = state
        .metadata
        .repositories()
        .apply_github_workflow_retention_page(now_unix, after, page_size)
        .await?;
    let repositories = page
        .deleted_runs
        .iter()
        .chain(&page.expired_runs)
        .map(|run| run.repo_id.as_str())
        .collect::<BTreeSet<_>>();
    for repo_id in repositories {
        match state.repository_incarnation(repo_id).await {
            Ok(Some(incarnation)) => {
                state.after_commit([AfterCommit::github_workflow_runs_expired(&incarnation)]);
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(
                repo_id,
                error = %error.operator_diagnostic(),
                "could not announce GitHub workflow run retention"
            ),
        }
    }
    Ok(page)
}

async fn apply_github_workflow_retention(
    state: &AppState,
    now_unix: u64,
) -> Result<GitHubWorkflowRetentionPass, ApiError> {
    let mut pass = GitHubWorkflowRetentionPass::default();
    let mut after = None;
    loop {
        let page =
            apply_github_workflow_retention_page(state, now_unix, after, RETENTION_BATCH_SIZE)
                .await?;
        pass.deleted_runs += page.deleted_runs.len();
        pass.expired_runs += page.expired_runs.len();
        after = page.next;
        if after.is_none() {
            break;
        }
    }
    pass.deleted_jobs_without_run = state
        .metadata
        .repositories()
        .delete_github_workflow_jobs_without_run(now_unix, RETENTION_BATCH_SIZE)
        .await?;
    Ok(pass)
}

impl BackgroundWork {
    pub(crate) fn start_retention(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "retention",
                cadence: Cadence::Fixed {
                    period: RETENTION_INTERVAL,
                    missed_ticks: MissedTickBehavior::Burst,
                },
                readiness: Readiness::Ungated,
            },
            |state| async move {
                let now_unix = unix_now()?;
                match apply_invite_retention(&state, now_unix).await {
                    Ok(0) => {}
                    Ok(invites) => {
                        tracing::info!(invites, "pruned ended repository invites");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to apply invite retention");
                    }
                }
                match apply_github_workflow_retention(&state, now_unix).await {
                    Ok(pass) if pass == GitHubWorkflowRetentionPass::default() => {}
                    Ok(pass) => {
                        tracing::info!(
                            deleted_runs = pass.deleted_runs,
                            expired_runs = pass.expired_runs,
                            deleted_jobs_without_run = pass.deleted_jobs_without_run,
                            "applied GitHub workflow run retention"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = %error.operator_diagnostic(),
                            "failed to apply GitHub workflow run retention"
                        );
                    }
                }
                Ok(())
            },
        );
    }
}
