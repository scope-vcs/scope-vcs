use crate::{error::ApiError, persistence::unix_now, state::AppState};
use serde::Serialize;
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct RequestRefCleanupDrainReport {
    pub(crate) attempted: usize,
    pub(crate) completed: usize,
    pub(crate) failed: Vec<RequestRefCleanupFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RequestRefCleanupFailure {
    pub(crate) request_id: String,
    pub(crate) error: String,
}

pub(crate) async fn drain_request_ref_cleanup(
    state: &AppState,
    now: u64,
) -> Result<RequestRefCleanupDrainReport, ApiError> {
    let store = state.metadata.cleanup();
    let pending = store.pending_request_ref_cleanups(Some(now)).await?;
    let mut report = RequestRefCleanupDrainReport::default();
    for cleanup in pending {
        report.attempted += 1;
        match crate::git::request_refs::cleanup_deleted_request_ref(
            state,
            &cleanup.incarnation,
            &cleanup.request_name,
            &cleanup.head_oid,
        )
        .await
        {
            Ok(()) => {
                store.complete_request_ref_cleanup(&cleanup.id).await?;
                report.completed += 1;
            }
            Err(error) => {
                let error = error.into_operator_diagnostic();
                store
                    .retry_request_ref_cleanup(&cleanup, now, error.clone())
                    .await?;
                report.failed.push(RequestRefCleanupFailure {
                    request_id: cleanup.request_id,
                    error,
                });
            }
        }
    }
    Ok(report)
}

impl AppState {
    pub(crate) fn start_request_ref_cleanup(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let result = match unix_now() {
                    Ok(now) => drain_request_ref_cleanup(&state, now).await,
                    Err(error) => Err(error),
                };
                match result {
                    Ok(report) if !report.failed.is_empty() => {
                        tracing::warn!(failed = ?report.failed, "request ref cleanup retained failed work")
                    }
                    Err(error) => tracing::warn!(?error, "request ref cleanup failed"),
                    Ok(_) => {}
                }
            }
        });
    }
}
