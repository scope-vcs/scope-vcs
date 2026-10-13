use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::{
        content_cleanup::drain_pending_source_blob_deletions,
        repository_collaboration::publish_committed_mutation,
    },
};
use std::time::Duration;
use tokio::time::MissedTickBehavior;

const RUN_RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;
const RETENTION_BATCH_SIZE: u64 = 100;
const RETENTION_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub(crate) async fn apply_run_retention(state: &AppState, now_unix: u64) -> anyhow::Result<usize> {
    let cutoff = now_unix.saturating_sub(RUN_RETENTION_SECONDS);
    let pruned = state
        .metadata
        .runs()
        .prune_terminal_runs(cutoff, now_unix, RETENTION_BATCH_SIZE)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let report = drain_pending_source_blob_deletions(state)
        .await
        .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
    if !report.failed_object_deletes.is_empty() {
        anyhow::bail!(
            "{} retained run source object(s) could not be deleted",
            report.failed_object_deletes.len()
        );
    }
    Ok(pruned)
}

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
                match apply_run_retention(&state, now_unix).await {
                    Ok(0) => {}
                    Ok(objects) => {
                        tracing::info!(source_objects = objects, "pruned expired run history");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to apply run retention");
                    }
                }
                match apply_invite_retention(&state, now_unix).await {
                    Ok(0) => {}
                    Ok(invites) => {
                        tracing::info!(invites, "pruned ended repository invites");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to apply invite retention");
                    }
                }
                Ok(())
            },
        );
    }
}
