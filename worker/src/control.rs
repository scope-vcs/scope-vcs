use crate::{
    execution::CloudExecutionCoordinator,
    health::{WorkerHealth, WorkerLoop},
    settings::{BATCH_SIZE, POLL_INTERVAL, WorkerSettings},
};
use scope_postgres::db::MetadataStore;

mod reconciliation;
use reconciliation::CloudReconciliation;

pub(crate) async fn run(
    metadata: MetadataStore,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    let execution = match settings.execution.clone() {
        Some(cloud) => Some(
            CloudExecutionCoordinator::new(metadata.clone(), cloud, settings.worker_id.clone())
                .await,
        ),
        None => None,
    };
    let mut cloud_reconciliation = CloudReconciliation::default();
    loop {
        if !super::schema_ready_or_wait(&metadata, &health).await {
            return Ok(());
        }
        let summary = match metadata
            .jobs()
            .run_ready_outbox_jobs(
                &settings.worker_id,
                BATCH_SIZE,
                &|| super::unix_now().map_err(|error| error.to_string()),
                &super::generate_persistence_id,
            )
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
        for run in &summary.created_runs {
            crate::run_events::publish_run_change(
                &metadata,
                &settings.worker_id,
                &run.repo_id,
                &run.run_id,
                scope_api_contract::RunChangeKind::Created,
            )
            .await;
        }
        if let Some(execution) = &execution {
            cloud_reconciliation.poll(execution);
        }
        health.mark_poll_succeeded(WorkerLoop::Control, super::unix_now()?);
        if summary.claimed >= BATCH_SIZE {
            continue;
        }
        if super::wait_or_shutdown(POLL_INTERVAL).await {
            return Ok(());
        }
    }
}
