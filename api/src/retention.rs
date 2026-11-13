use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{
    persistence::unix_now, repo_events::RepoChangeReason, state::AppState,
    use_cases::repository_collaboration::publish_committed_mutation,
};
use std::time::Duration;
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
                Ok(())
            },
        );
    }
}
