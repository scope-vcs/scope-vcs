use crate::{
    background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness},
    error::ApiError,
    persistence::unix_now,
    state::AppState,
};
use std::time::Duration;

const SOURCE_BLOB_CLEANUP_INTERVAL: Duration = Duration::from_secs(1);

impl BackgroundWork {
    pub(crate) fn start_source_blob_cleanup(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "orphan object cleanup",
                cadence: Cadence::AfterPass {
                    delay: SOURCE_BLOB_CLEANUP_INTERVAL,
                    wakeup: None,
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { drain_orphan_objects(&state).await },
        );
    }
}

async fn drain_orphan_objects(state: &AppState) -> Result<(), ApiError> {
    let report = scope_content_lifecycle::drain_source_blob_cleanup(
        &state.metadata,
        state.object_store.as_ref(),
        unix_now()?,
    )
    .await?;
    for failure in &report.failed_object_deletes {
        tracing::warn!(
            object_key = %scope_storage::object_key(&failure.blob),
            error = %failure.error.message,
            "failed to delete orphan object"
        );
    }
    if report.attempted > 0 {
        tracing::info!(
            attempted = report.attempted,
            deleted = report.deleted,
            retained = report.retained,
            "processed orphan object jobs"
        );
    }
    Ok(())
}
