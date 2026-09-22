use super::ecs::{EcsClient, RejectionReason, StartError, StopOutcome};
use super::provisioning::Provisioning;
use crate::settings::{BATCH_SIZE, CloudExecutionSettings};
use anyhow::Context as _;
use scope_domain::runs::{exit_code::SetupFailure, step::AttemptConclusion};
use scope_postgres::db::MetadataStore;
use scope_product_analytics::ProductAnalytics;
use sha2::{Digest as _, Sha256};
use std::time::Duration;

const DISPATCH_LEASE: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub(crate) struct CloudExecutionCoordinator {
    metadata: MetadataStore,
    product_analytics: ProductAnalytics,
    ecs: EcsClient,
    origin_id: String,
    settings: CloudExecutionSettings,
}

impl CloudExecutionCoordinator {
    pub(crate) async fn new(
        metadata: MetadataStore,
        product_analytics: ProductAnalytics,
        settings: CloudExecutionSettings,
        origin_id: String,
    ) -> Self {
        Self {
            metadata,
            product_analytics,
            ecs: EcsClient::new(settings.clone()).await,
            origin_id,
            settings,
        }
    }

    pub(crate) async fn dispatch_available(&self, now_unix: u64) -> anyhow::Result<usize> {
        for run in self
            .metadata
            .runs()
            .expire_capacity_retries(now_unix, BATCH_SIZE as u64)
            .await
            .map_err(db_error)?
        {
            crate::run_events::publish_run_change(
                &self.metadata,
                &self.origin_id,
                run.workflow.repository_id(),
                &run.id,
                scope_api_contract::RunChangeKind::StatusChanged,
            )
            .await;
        }
        if self.settings.max_concurrency == 0 {
            return Ok(0);
        }
        let mut starts = Provisioning::new(self.settings.max_concurrency);
        let dispatch_result: anyhow::Result<usize> = async {
            let mut dispatched = 0;
            // Bound each coordinator tick, including exhausted-job repairs and contention.
            for _ in 0..self.settings.max_concurrency.max(1) {
                starts.wait_for_slot().await?;
                let attempt_id = crate::random_hex("attempt_", 16)?;
                let bootstrap_token = crate::random_hex("scope_bootstrap_", 32)?;
                let bootstrap_hash = hex::encode(Sha256::digest(bootstrap_token.as_bytes()));
                let claim = match self
                    .metadata
                    .runs()
                    .admit_next_job(
                        self.settings.max_concurrency as u64,
                        &attempt_id,
                        &bootstrap_hash,
                        &self.settings.runtime_version,
                        now_unix,
                        now_unix + DISPATCH_LEASE.as_secs(),
                    )
                    .await
                    .map_err(db_error)?
                {
                    scope_postgres::db::DispatchAdmission::Admitted(claim) => *claim,
                    scope_postgres::db::DispatchAdmission::Contended => continue,
                    scope_postgres::db::DispatchAdmission::AtCapacity
                    | scope_postgres::db::DispatchAdmission::Empty => break,
                };
                self.product_analytics.capture_workflow_attempt_started(
                    claim.repository.incarnation_id(),
                    &claim.run,
                    &claim.attempt,
                );
                self.publish_status_change(&claim).await;
                let execution = self.clone();
                starts.spawn(async move {
                    execution
                        .provision(claim, bootstrap_token, bootstrap_hash, now_unix)
                        .await
                });
                dispatched += 1;
            }
            Ok(dispatched)
        }
        .await;
        // A failed admission or launch must not cancel other already-reserved starts.
        let provision_result = starts.finish().await;
        let dispatched = dispatch_result?;
        provision_result?;
        Ok(dispatched)
    }

    async fn provision(
        &self,
        claim: scope_postgres::db::DispatchClaim,
        bootstrap_token: String,
        bootstrap_hash: String,
        now_unix: u64,
    ) -> anyhow::Result<()> {
        let attempt_id = &claim.attempt.id;
        match self.ecs.start(attempt_id, &bootstrap_token).await {
            Ok(external_run_id) => {
                self.metadata
                    .runs()
                    .record_external_run_id(attempt_id, &external_run_id)
                    .await
                    .map_err(db_error)?;
                self.publish_status_change(&claim).await;
                tracing::info!(attempt_id = %attempt_id, external_run_id, run_id = %claim.run.id, job = %claim.job.key.as_str(), "dispatched cloud run");
            }
            Err(StartError::Rejected { reason, error }) => {
                let now_unix = crate::unix_now()?.max(now_unix);
                let message = format!("provider rejected dispatch: {error}")
                    .chars()
                    .take(2048)
                    .collect::<String>();
                // Only the broker's confirmed capacity rejection can request another attempt.
                // Quota and permanent failures require an operator/configuration change.
                let mutation = match reason {
                    RejectionReason::Capacity => {
                        self.metadata
                            .runs()
                            .reject_capacity_attempt(
                                attempt_id,
                                &bootstrap_hash,
                                &message,
                                now_unix,
                            )
                            .await
                    }
                    _ => {
                        self.metadata
                            .runs()
                            .complete_attempt(
                                attempt_id,
                                &bootstrap_hash,
                                AttemptConclusion::SetupFailed {
                                    exit_code: SetupFailure::ProviderRejected.exit_code(),
                                    message,
                                },
                                false,
                                now_unix,
                            )
                            .await
                    }
                }
                .map_err(db_error)?;
                if let Some(due) = mutation
                    .claim
                    .job
                    .capacity_retry
                    .as_ref()
                    .and_then(|retry| retry.next_attempt_at_unix)
                {
                    tracing::info!(attempt_id = %attempt_id, retry_at_unix = due,
                        "ECS capacity unavailable; scheduled another attempt");
                } else {
                    tracing::error!(attempt_id = %attempt_id, ?reason, error = %error,
                        "ECS rejected cloud run; no capacity retry scheduled");
                }
                capture_attempt_completed(&self.product_analytics, &mutation);
                self.metadata
                    .runs()
                    .complete_cloud_task_absence(attempt_id, now_unix)
                    .await
                    .map_err(db_error)?;
                self.publish_status_change(&mutation.claim).await;
            }
            Err(StartError::Ambiguous(error)) => {
                tracing::warn!(attempt_id = %attempt_id, error = %error, "ECS dispatch outcome is ambiguous; lease recovery and task cleanup own resolution");
            }
        }
        Ok(())
    }

    pub(crate) async fn abort_canceled(&self, now_unix: u64) -> anyhow::Result<usize> {
        let attempts = self
            .metadata
            .runs()
            .claim_cloud_attempt_aborts(
                now_unix,
                self.settings.max_concurrency.max(BATCH_SIZE) as u64,
            )
            .await
            .map_err(db_error)?;
        let mut tasks = tokio::task::JoinSet::new();
        for attempt in attempts {
            let metadata = self.metadata.clone();
            let ecs = self.ecs.clone();
            let product_analytics = self.product_analytics.clone();
            tasks.spawn(async move {
                abort_canceled_attempt(metadata, product_analytics, ecs, attempt, now_unix).await
            });
        }
        let mut aborted = 0;
        while let Some(result) = tasks.join_next().await {
            if let Some(claim) = result.context("cloud cancellation task panicked")?? {
                self.publish_status_change(&claim).await;
                aborted += 1;
            }
        }
        Ok(aborted)
    }

    pub(crate) async fn cleanup_terminal(&self, now_unix: u64) -> anyhow::Result<usize> {
        let tasks = self
            .metadata
            .runs()
            .claim_terminal_cloud_task_stops(
                now_unix,
                self.settings.max_concurrency.max(BATCH_SIZE) as u64,
            )
            .await
            .map_err(db_error)?;
        let mut reconciliations = tokio::task::JoinSet::new();
        for task in tasks {
            let metadata = self.metadata.clone();
            let ecs = self.ecs.clone();
            reconciliations
                .spawn(async move { cleanup_terminal_task(metadata, ecs, task, now_unix).await });
        }
        let mut stopped = 0;
        while let Some(result) = reconciliations.join_next().await {
            if result.context("terminal cloud cleanup task panicked")?? {
                stopped += 1;
            }
        }
        Ok(stopped)
    }

    async fn publish_status_change(&self, claim: &scope_postgres::db::DispatchClaim) {
        crate::run_events::publish_run_change_for(
            &self.metadata,
            &self.origin_id,
            &claim.repository,
            &claim.run.id,
            scope_api_contract::RunChangeKind::StatusChanged,
        )
        .await;
    }
}

fn capture_attempt_completed(
    product_analytics: &ProductAnalytics,
    mutation: &scope_postgres::db::AttemptMutation,
) {
    if let Some(claim) = mutation.transition() {
        product_analytics.capture_workflow_attempt_completed(
            claim.repository.incarnation_id(),
            &claim.run,
            &claim.attempt,
        );
    }
}

async fn abort_canceled_attempt(
    metadata: MetadataStore,
    product_analytics: ProductAnalytics,
    ecs: EcsClient,
    attempt: scope_postgres::db::CloudTaskStop,
    now_unix: u64,
) -> anyhow::Result<Option<scope_postgres::db::DispatchClaim>> {
    match ecs.stop_terminal_task(&attempt.attempt_id).await {
        Ok(StopOutcome::Stopped) => {
            let mutation = metadata
                .runs()
                .confirm_provider_cancellation(&attempt.attempt_id, now_unix)
                .await
                .map_err(db_error)?;
            capture_attempt_completed(&product_analytics, &mutation);
            metadata
                .runs()
                .complete_cloud_task_stop(&attempt.attempt_id, now_unix)
                .await
                .map_err(db_error)?;
            tracing::info!(attempt_id = %attempt.attempt_id, external_run_id = ?attempt.external_run_id, "aborted canceled cloud run");
            Ok(Some(mutation.claim))
        }
        Ok(StopOutcome::Stopping { stuck }) => {
            metadata
                .runs()
                .release_cloud_task_stop_claim(&attempt.attempt_id)
                .await
                .map_err(db_error)?;
            if stuck {
                tracing::warn!(attempt_id = %attempt.attempt_id, "cloud run cancellation is still stopping after 15 minutes");
            }
            Ok(None)
        }
        Err(error) => {
            metadata
                .runs()
                .release_cloud_task_stop_claim(&attempt.attempt_id)
                .await
                .map_err(db_error)?;
            tracing::warn!(attempt_id = %attempt.attempt_id, error = %error, "failed to abort canceled cloud run; will retry");
            Ok(None)
        }
    }
}

async fn cleanup_terminal_task(
    metadata: MetadataStore,
    ecs: EcsClient,
    task: scope_postgres::db::CloudTaskStop,
    now_unix: u64,
) -> anyhow::Result<bool> {
    match ecs.stop_terminal_task(&task.attempt_id).await {
        Ok(StopOutcome::Stopped) => {
            metadata
                .runs()
                .complete_cloud_task_stop(&task.attempt_id, now_unix)
                .await
                .map_err(db_error)?;
            tracing::info!(attempt_id = %task.attempt_id, "reconciled terminal cloud task");
            Ok(true)
        }
        Ok(StopOutcome::Stopping { stuck }) => {
            metadata
                .runs()
                .release_cloud_task_stop_claim(&task.attempt_id)
                .await
                .map_err(db_error)?;
            if stuck {
                tracing::warn!(attempt_id = %task.attempt_id, "terminal cloud task is still stopping after 15 minutes");
            }
            Ok(false)
        }
        Err(error) => {
            metadata
                .runs()
                .release_cloud_task_stop_claim(&task.attempt_id)
                .await
                .map_err(db_error)?;
            tracing::warn!(attempt_id = %task.attempt_id, error = %error, "failed to reconcile terminal cloud task; will retry");
            Ok(false)
        }
    }
}

fn db_error(error: scope_postgres::error::PostgresError) -> anyhow::Error {
    anyhow::anyhow!(error.message)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod capacity_tests;
