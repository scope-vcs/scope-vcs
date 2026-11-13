use crate::{
    health::{WorkerHealth, WorkerLoop},
    settings::{CONTROL_POLL_BATCH_SIZE, POLL_INTERVAL, WorkerSettings},
};
use scope_postgres::db::MetadataStore;

pub(crate) async fn run(
    metadata: MetadataStore,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    loop {
        if !super::schema_ready_or_wait(&metadata, &health).await {
            return Ok(());
        }
        let summary = match metadata
            .jobs()
            .run_ready_outbox_jobs(&settings.worker_id, CONTROL_POLL_BATCH_SIZE, &|| {
                super::unix_now().map_err(|error| error.to_string())
            })
            .await
        {
            Ok(summary) => summary,
            Err(error) => {
                tracing::error!(error = %error.message, "control work failed; retrying");
                if super::wait_or_shutdown(POLL_INTERVAL).await {
                    return Ok(());
                }
                continue;
            }
        };
        if summary.claimed > 0 {
            tracing::info!(
                claimed = summary.claimed,
                completed = summary.completed,
                failed = summary.failed,
                "processed outbox jobs"
            );
        }
        health.mark_poll_succeeded(WorkerLoop::Control, super::unix_now()?);
        if summary.claimed >= CONTROL_POLL_BATCH_SIZE {
            continue;
        }
        if super::wait_or_shutdown(POLL_INTERVAL).await {
            return Ok(());
        }
    }
}
