use super::{
    RunStore, entities,
    run_attempt_persistence::{locked_jobs, locked_run, save_jobs, save_run},
    runs::workflow_revision_for_run,
};
use crate::error::PostgresError;
use scope_domain::runs::{job::reconcile_run, run::Run};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, TransactionTrait};

impl RunStore {
    /// Settle queued capacity retries whose two-minute window has elapsed.
    /// Dispatch calls this before admission; each row is rechecked under the run locks.
    pub async fn expire_capacity_retries(
        &self,
        now_unix: u64,
        limit: u64,
    ) -> Result<Vec<Run>, PostgresError> {
        let cutoff =
            now_unix.saturating_sub(scope_domain::runs::job::CAPACITY_RETRY_WINDOW_SECONDS);
        let candidates = entities::run_job::Entity::find()
            .filter(entities::run_job::Column::State.eq("queued"))
            .filter(
                entities::run_job::Column::CapacityRetryFirstRejectedAtUnix.lte(
                    super::integer_columns::u64_to_i64(cutoff, "capacity retry cutoff")?,
                ),
            )
            .order_by_asc(entities::run_job::Column::CapacityRetryFirstRejectedAtUnix)
            .limit(limit)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        let mut settled = Vec::new();
        for candidate in candidates {
            let tx = self.db.begin().await.map_err(PostgresError::internal)?;
            let active_auto_merge =
                super::request_auto_merge::lock_active_auto_merge_for_run(&tx, &candidate.run_id)
                    .await?;
            let mut jobs = locked_jobs(&tx, &candidate.run_id).await?;
            let mut run = locked_run(&tx, &candidate.run_id).await?;
            let Some(job) = jobs
                .iter_mut()
                .find(|job| job.key.as_str() == candidate.job_key)
            else {
                return Err(PostgresError::internal_message(
                    "capacity retry job is missing",
                ));
            };
            if job
                .expire_capacity_retry(now_unix)
                .map_err(PostgresError::from)?
            {
                let revision = workflow_revision_for_run(&tx, &run).await?;
                reconcile_run(&mut run, &mut jobs, &revision, now_unix)
                    .map_err(PostgresError::from)?;
                save_jobs(&tx, &jobs).await?;
                save_run(&tx, &run).await?;
                super::request_auto_merge::stop_auto_merge_for_terminal_run(
                    &tx,
                    active_auto_merge,
                    &run,
                )
                .await?;
                settled.push(run);
            }
            tx.commit().await.map_err(PostgresError::internal)?;
        }
        Ok(settled)
    }
}
