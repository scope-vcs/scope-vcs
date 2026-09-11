use super::{
    DispatchClaim, RunStore,
    run_attempt_persistence::{locked_jobs, locked_run},
};
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, Statement, TransactionTrait};

#[derive(Debug)]
pub enum DispatchAdmission {
    Admitted(Box<DispatchClaim>),
    AtCapacity,
    Contended,
    Empty,
}

pub(super) async fn lock_admission(tx: &DatabaseTransaction) -> Result<(), PostgresError> {
    tx.execute(Statement::from_string(DatabaseBackend::Postgres,
        "SELECT pg_advisory_xact_lock(hashtextextended('scope:cloud-admission:' || current_schema(), 0))"))
        .await.map_err(PostgresError::internal)?;
    Ok(())
}

impl RunStore {
    /// Count, select, transition, and persist under one global admission lock.
    #[allow(clippy::too_many_arguments)]
    pub async fn admit_next_job(
        &self,
        max_concurrency: u64,
        attempt_id: &str,
        token_hash: &str,
        runtime_version: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<DispatchAdmission, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        lock_admission(&tx).await?;
        let row = tx.query_one(Statement::from_string(DatabaseBackend::Postgres,
            "SELECT COUNT(*)::bigint AS count FROM scope_run_attempts WHERE state IN ('dispatching', 'running')"))
            .await.map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("active attempt count is missing"))?;
        let active = u64::try_from(
            row.try_get::<i64>("", "count")
                .map_err(PostgresError::internal)?,
        )
        .map_err(PostgresError::internal)?;
        if active >= max_concurrency {
            return Ok(DispatchAdmission::AtCapacity);
        }
        let Some(row) = tx
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT job.run_id, job.job_key FROM scope_run_jobs job
             JOIN scope_runs run ON run.id = job.run_id
             WHERE job.state = 'queued'
               AND run.state IN ('queued', 'dispatching', 'running')
               AND run.cancellation_requested = FALSE
               AND NOT EXISTS (
                 SELECT 1 FROM scope_run_attempts previous
                 WHERE previous.run_id = job.run_id AND previous.job_key = job.job_key
                   AND previous.state IN ('succeeded', 'failed', 'canceled', 'lost')
                   AND previous.runner_stop_completed_at_unix IS NULL
               )
             ORDER BY job.created_at_unix, job.run_id, job.job_key
             LIMIT 1",
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(DispatchAdmission::Empty);
        };
        let run_id = row
            .try_get::<String>("", "run_id")
            .map_err(PostgresError::internal)?;
        let job_key = row
            .try_get::<String>("", "job_key")
            .map_err(PostgresError::internal)?;
        // Match cancellation/completion lock order: all jobs, then run, then attempt.
        let jobs = locked_jobs(&tx, &run_id).await?;
        let run = locked_run(&tx, &run_id).await?;
        let job = jobs
            .iter()
            .find(|job| job.key.as_str() == job_key)
            .ok_or_else(|| PostgresError::internal_message("admission job is missing"))?;
        if job.state != scope_domain::runs::job::RunJobState::Queued
            || run.cancellation_requested
            || run.state.is_terminal()
        {
            return Ok(DispatchAdmission::Contended);
        }
        let claim = self
            .dispatch_in_transaction(
                &tx,
                &run_id,
                &job_key,
                attempt_id,
                token_hash,
                runtime_version,
                now_unix,
                lease_expires_at_unix,
            )
            .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DispatchAdmission::Admitted(Box::new(claim)))
    }
}

#[cfg(test)]
mod tests;
