use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{error::ApiError, persistence::unix_now, state::AppState};
use std::time::Duration;
use tokio::time::MissedTickBehavior;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RequestRefCleanupDrainReport {
    pub(crate) attempted: usize,
    pub(crate) completed: usize,
    pub(crate) failed: Vec<RequestRefCleanupFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestRefCleanupFailure {
    pub(crate) request_id: String,
    pub(crate) error: String,
}

pub(crate) async fn drain_request_ref_cleanup(
    state: &AppState,
) -> Result<RequestRefCleanupDrainReport, ApiError> {
    drain_request_ref_cleanup_at(state, unix_now()?).await
}

pub(crate) async fn drain_request_ref_cleanup_at(
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

impl BackgroundWork {
    pub(crate) fn start_request_ref_cleanup(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "request ref cleanup",
                cadence: Cadence::Fixed {
                    period: Duration::from_secs(30),
                    missed_ticks: MissedTickBehavior::Skip,
                },
                readiness: Readiness::Ungated,
            },
            |state| async move {
                let report = drain_request_ref_cleanup(&state).await?;
                if !report.failed.is_empty() {
                    tracing::warn!(failed = ?report.failed, "request ref cleanup retained failed work");
                }
                Ok(())
            },
        );
    }
}
