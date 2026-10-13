use super::{
    AttemptMutation, DispatchClaim, RunStore,
    run_attempt_persistence::{
        attempt_run_id, locked_attempt, locked_attempt_context, locked_attempt_steps, locked_jobs,
        locked_run, run_repository, save_attempt, save_attempt_steps, save_job, save_jobs,
        save_run,
    },
    runs::workflow_revision_for_run,
};
use crate::error::PostgresError;
use scope_domain::{
    error::DomainError,
    runs::{
        attempt::RunAttempt,
        job::{RunJob, reconcile_run},
        run::Run,
        step::RunAttemptStep,
        workflow::revision::WorkflowRevision,
    },
};
use sea_orm::{DatabaseTransaction, TransactionTrait};

impl RunStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_runtime"))]
    pub async fn claim_runtime(
        &self,
        attempt_id: &str,
        bootstrap_token_hash: &str,
        attempt_token_hash: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await?;
        let (run, job, mut attempt) = locked_attempt_context(&tx, attempt_id).await?;
        let steps = locked_attempt_steps(&tx, attempt_id).await?;
        attempt
            .claim_runtime(
                &job,
                bootstrap_token_hash,
                attempt_token_hash,
                now_unix,
                lease_expires_at_unix,
            )
            .map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        let claim = dispatch_claim(&tx, run, job, attempt, steps, workflow_revision).await?;
        tx.commit().await?;
        Ok(claim)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "authenticate_attempt"))]
    pub async fn authenticate_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await?;
        let (run, job, attempt) = locked_attempt_context(&tx, attempt_id).await?;
        let steps = locked_attempt_steps(&tx, attempt_id).await?;
        attempt
            .authenticate(&job, token_hash, now_unix)
            .map_err(PostgresError::from)?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        let claim = dispatch_claim(&tx, run, job, attempt, steps, workflow_revision).await?;
        tx.commit().await?;
        Ok(claim)
    }

    pub(super) async fn mutate_attempt(
        &self,
        attempt_id: &str,
        mutate: impl FnOnce(
            &Run,
            &mut RunJob,
            &mut RunAttempt,
            &mut [RunAttemptStep],
        ) -> Result<(), DomainError>,
    ) -> Result<AttemptMutation, PostgresError> {
        let tx = self.db.begin().await?;
        let guard_run_id = attempt_run_id(&tx, attempt_id).await?;
        let active_auto_merge =
            super::request_auto_merge::lock_active_auto_merge_for_run(&tx, &guard_run_id).await?;
        let mut jobs = locked_jobs(&tx, &guard_run_id).await?;
        let mut run = locked_run(&tx, &guard_run_id).await?;
        let mut attempt = locked_attempt(&tx, attempt_id).await?;
        let mut steps = locked_attempt_steps(&tx, attempt_id).await?;
        let job_index = jobs
            .iter()
            .position(|job| job.key == attempt.job_key)
            .ok_or_else(|| PostgresError::internal_message("run attempt job is missing"))?;
        let transitioned = !attempt.state.is_terminal();
        let job = &mut jobs[job_index];
        mutate(&run, job, &mut attempt, &mut steps).map_err(PostgresError::from)?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        let transition_time = attempt.completed_at_unix.unwrap_or(job.updated_at_unix);
        reconcile_run(&mut run, &mut jobs, &workflow_revision, transition_time)
            .map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        save_attempt_steps(&tx, &steps).await?;
        save_jobs(&tx, &jobs).await?;
        save_run(&tx, &run).await?;
        super::request_auto_merge::stop_auto_merge_for_terminal_run(&tx, active_auto_merge, &run)
            .await?;
        let job = jobs.swap_remove(job_index);
        let claim = dispatch_claim(&tx, run, job, attempt, steps, workflow_revision).await?;
        tx.commit().await?;
        Ok(AttemptMutation {
            claim,
            transitioned,
        })
    }

    pub(super) async fn mutate_active_attempt(
        &self,
        attempt_id: &str,
        mutate: impl FnOnce(
            &Run,
            &mut RunJob,
            &mut RunAttempt,
            &mut [RunAttemptStep],
        ) -> Result<(), DomainError>,
    ) -> Result<DispatchClaim, PostgresError> {
        let tx = self.db.begin().await?;
        let guard_run_id = attempt_run_id(&tx, attempt_id).await?;
        let active_auto_merge =
            super::request_auto_merge::lock_active_auto_merge_for_run(&tx, &guard_run_id).await?;
        let (run_snapshot, mut job, mut attempt) = locked_attempt_context(&tx, attempt_id).await?;
        let mut steps = locked_attempt_steps(&tx, attempt_id).await?;
        let mut run = locked_run(&tx, &run_snapshot.id).await?;
        mutate(&run, &mut job, &mut attempt, &mut steps).map_err(PostgresError::from)?;
        save_job(&tx, &job).await?;
        save_attempt(&tx, &attempt).await?;
        save_attempt_steps(&tx, &steps).await?;
        let workflow_revision = workflow_revision_for_run(&tx, &run).await?;
        let mut jobs = super::run_operations::required_run_jobs(&tx, &run.id).await?;
        if let Some(stored) = jobs.iter_mut().find(|stored| stored.key == job.key) {
            *stored = job.clone();
        }
        reconcile_run(&mut run, &mut jobs, &workflow_revision, job.updated_at_unix)
            .map_err(PostgresError::from)?;
        save_run(&tx, &run).await?;
        super::request_auto_merge::stop_auto_merge_for_terminal_run(&tx, active_auto_merge, &run)
            .await?;
        let claim = dispatch_claim(&tx, run, job, attempt, steps, workflow_revision).await?;
        tx.commit().await?;
        Ok(claim)
    }
}

pub(super) async fn dispatch_claim(
    tx: &DatabaseTransaction,
    run: Run,
    job: RunJob,
    attempt: RunAttempt,
    steps: Vec<RunAttemptStep>,
    workflow_revision: WorkflowRevision,
) -> Result<DispatchClaim, PostgresError> {
    Ok(DispatchClaim {
        repository: run_repository(tx, &run).await?,
        run,
        job,
        attempt,
        steps,
        workflow_revision,
    })
}
