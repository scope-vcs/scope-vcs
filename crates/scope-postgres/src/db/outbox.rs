#[cfg(test)]
use super::MetadataStore;
use super::{
    GeneratedIdKind, JobStore, acquire_aggregate_lock, entities, generated_ids::generate_id,
    integer_columns, projection_read_models::fold_live_projection_read_models,
};
use crate::{error::PostgresError, job_trace::QueuedTrace};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseTransaction, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
    sea_query::{Expr, LockBehavior, LockType, OnConflict},
};
use std::sync::Arc;
use tracing::Instrument as _;

const PROJECTION_READ_MODEL_REBUILD: &str = "projection_read_model_rebuild";
const JOB_READY: &str = "ready";
const JOB_RUNNING: &str = "running";
const JOB_SUCCEEDED: &str = "succeeded";
const JOB_FAILED: &str = "failed";
const DEFAULT_JOB_LEASE_SECS: i64 = 60;
const MAX_RETRY_DELAY_SECS: i64 = 300;
const MAX_JOB_ATTEMPTS: i64 = 12;
const SUCCEEDED_JOB_RETENTION_SECS: i64 = 7 * 24 * 60 * 60;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutboxRunSummary {
    pub claimed: usize,
    pub completed: usize,
    pub failed: usize,
    pub created_runs: Vec<OutboxCreatedRun>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxCreatedRun {
    pub repo_id: String,
    pub run_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutboxJobCounts {
    pub ready: usize,
    pub running: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Clone, Debug)]
pub(super) struct ClaimedOutboxJob {
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) repo_id: String,
    pub(super) repo_version: i64,
    pub(super) attempts: i64,
    pub(super) queued_trace: QueuedTrace,
}

fn outbox_time(
    current_time: &dyn Fn() -> Result<u64, String>,
) -> Result<(u64, i64), PostgresError> {
    let now_unix = current_time().map_err(|error| {
        PostgresError::internal_message(format!("outbox clock failed: {error}"))
    })?;
    Ok((now_unix, unix_timestamp_i64(now_unix)?))
}

impl JobStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "run_ready_outbox_jobs"))]
    pub async fn run_ready_outbox_jobs(
        &self,
        worker_id: &str,
        limit: usize,
        current_time: &dyn Fn() -> Result<u64, String>,
    ) -> Result<OutboxRunSummary, PostgresError> {
        if limit == 0 {
            return Ok(OutboxRunSummary::default());
        }

        let db = Arc::clone(&self.db);
        let mut summary = OutboxRunSummary::default();
        for _ in 0..limit {
            let (claim_now_unix, claim_now) = outbox_time(current_time)?;
            let Some(job) =
                claim_next_ready_job(db.as_ref(), worker_id, DEFAULT_JOB_LEASE_SECS, claim_now)
                    .await?
            else {
                break;
            };
            summary.claimed += 1;
            let span = tracing::info_span!(
                parent: None,
                "job.outbox",
                otel.kind = "consumer",
                scope.job.kind = %job.kind,
                scope.job.id = %job.id,
                scope.job.attempt = job.attempts.saturating_add(1),
            );
            job.queued_trace.link(&span);

            match execute_outbox_job(db.as_ref(), &job, claim_now_unix)
                .instrument(span.clone())
                .await
            {
                Ok(created_runs) => {
                    let (_, completion_now) = outbox_time(current_time)?;
                    complete_outbox_job(db.as_ref(), &job, worker_id, completion_now)
                        .instrument(span)
                        .await?;
                    summary.completed += 1;
                    summary.created_runs.extend(created_runs);
                }
                Err(error) => {
                    let message = error.message;
                    let (_, completion_now) = outbox_time(current_time)?;
                    let attempts = next_retry_attempt(job.attempts)?;
                    let entered = span.enter();
                    if is_terminal_retry_attempt(attempts) {
                        tracing::error!(
                            job_id = %job.id,
                            kind = %job.kind,
                            attempts,
                            max_attempts = MAX_JOB_ATTEMPTS,
                            error = %message,
                            "outbox job exhausted retries; marking failed"
                        );
                    } else {
                        tracing::warn!(
                            job_id = %job.id,
                            kind = %job.kind,
                            attempts,
                            max_attempts = MAX_JOB_ATTEMPTS,
                            next_retry_delay_secs = retry_delay_seconds(attempts),
                            error = %message,
                            "outbox job failed; scheduling retry"
                        );
                    }
                    drop(entered);
                    fail_outbox_job(db.as_ref(), &job, worker_id, message, completion_now)
                        .instrument(span)
                        .await?;
                    summary.failed += 1;
                }
            }
        }
        Ok(summary)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn outbox_job_counts_for_tests(&self) -> Result<OutboxJobCounts, PostgresError> {
        let rows = entities::outbox_job::Entity::find()
            .all(self.db.as_ref())
            .await?;
        let mut counts = OutboxJobCounts {
            total: rows.len(),
            ..OutboxJobCounts::default()
        };
        for row in rows {
            match row.state.as_str() {
                JOB_READY => counts.ready += 1,
                JOB_RUNNING => counts.running += 1,
                JOB_SUCCEEDED => counts.succeeded += 1,
                JOB_FAILED => counts.failed += 1,
                _ => {}
            }
        }
        Ok(counts)
    }
}

pub async fn enqueue_projection_read_model_rebuild<C>(
    conn: &C,
    repo_id: &str,
    repo_version: u64,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let job = entities::outbox_job::Model::projection_read_model_rebuild(
        generate_id(GeneratedIdKind::OutboxJob)?,
        repo_id,
        repo_version,
        now_unix,
    )?;
    entities::outbox_job::Entity::insert(job.into_active_model())
        .on_conflict(
            OnConflict::column(entities::outbox_job::Column::IdempotencyKey)
                .do_nothing()
                .to_owned(),
        )
        .try_insert()
        .exec(conn)
        .await?;
    Ok(())
}

async fn claim_next_ready_job<C>(
    conn: &C,
    worker_id: &str,
    lease_seconds: i64,
    now: i64,
) -> Result<Option<ClaimedOutboxJob>, PostgresError>
where
    C: ConnectionTrait + TransactionTrait<Transaction = DatabaseTransaction>,
{
    let tx = conn.begin().await?;
    let runnable = Condition::any()
        .add(
            Condition::all()
                .add(entities::outbox_job::Column::State.eq(JOB_READY))
                .add(entities::outbox_job::Column::NextRunAtUnix.lte(now)),
        )
        .add(
            Condition::all()
                .add(entities::outbox_job::Column::State.eq(JOB_RUNNING))
                .add(entities::outbox_job::Column::LeaseExpiresAtUnix.lte(now)),
        );

    let Some(job) = entities::outbox_job::Entity::find()
        .filter(entities::outbox_job::Column::CompletedAtUnix.is_null())
        .filter(runnable)
        .order_by_asc(entities::outbox_job::Column::NextRunAtUnix)
        .order_by_asc(entities::outbox_job::Column::CreatedAtUnix)
        .order_by_asc(entities::outbox_job::Column::Id)
        .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
        .one(&tx)
        .await?
    else {
        tx.commit().await?;
        return Ok(None);
    };

    let claimed = entities::outbox_job::Entity::update_many()
        .filter(entities::outbox_job::Column::Id.eq(job.id.clone()))
        .filter(entities::outbox_job::Column::CompletedAtUnix.is_null())
        .col_expr(
            entities::outbox_job::Column::State,
            Expr::value(JOB_RUNNING),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseOwner,
            Expr::value(Some(worker_id.to_string())),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseExpiresAtUnix,
            Expr::value(Some(now.checked_add(lease_seconds).ok_or_else(|| {
                PostgresError::internal_message("outbox lease expiry exceeds i64 range")
            })?)),
        )
        .col_expr(
            entities::outbox_job::Column::UpdatedAtUnix,
            Expr::value(now),
        )
        .exec(&tx)
        .await?;
    if claimed.rows_affected != 1 {
        tx.rollback().await?;
        return Ok(None);
    }
    tx.commit().await?;

    Ok(Some(ClaimedOutboxJob {
        id: job.id,
        kind: job.kind,
        repo_id: job.repo_id,
        repo_version: job.repo_version,
        attempts: job.attempts,
        queued_trace: QueuedTrace::from_column(job.traceparent),
    }))
}

async fn execute_outbox_job<C>(
    conn: &C,
    job: &ClaimedOutboxJob,
    now_unix: u64,
) -> Result<Vec<OutboxCreatedRun>, PostgresError>
where
    C: ConnectionTrait + TransactionTrait<Transaction = DatabaseTransaction>,
{
    match job.kind.as_str() {
        PROJECTION_READ_MODEL_REBUILD => {
            rebuild_live_projection_read_models_for_job(conn, job).await?;
            Ok(Vec::new())
        }
        super::push_triggers::JOB_KIND => {
            let run_ids = super::push_triggers::evaluate(conn, job, now_unix).await?;
            Ok(run_ids
                .into_iter()
                .map(|run_id| OutboxCreatedRun {
                    repo_id: job.repo_id.clone(),
                    run_id,
                })
                .collect())
        }
        kind => Err(PostgresError::internal_message(format!(
            "unknown outbox job kind {kind}"
        ))),
    }
}

async fn rebuild_live_projection_read_models_for_job<C>(
    conn: &C,
    job: &ClaimedOutboxJob,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait + TransactionTrait<Transaction = DatabaseTransaction>,
{
    let tx = conn.begin().await?;
    acquire_aggregate_lock(&tx, "repository", &job.repo_id).await?;
    let Some(repo) = entities::repository::Entity::find_by_id(job.repo_id.clone())
        .one(&tx)
        .await?
    else {
        tx.commit().await?;
        return Ok(());
    };
    if repo.content_version != job.repo_version {
        tx.commit().await?;
        return Ok(());
    }
    let content_version =
        integer_columns::i64_to_u64(repo.content_version, "repository content version")?;
    fold_live_projection_read_models(&tx, &repo.id, content_version).await?;
    tx.commit().await?;
    Ok(())
}

async fn complete_outbox_job<C>(
    conn: &C,
    job: &ClaimedOutboxJob,
    worker_id: &str,
    now: i64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let completed = entities::outbox_job::Entity::update_many()
        .filter(entities::outbox_job::Column::Id.eq(job.id.clone()))
        .filter(entities::outbox_job::Column::LeaseOwner.eq(worker_id.to_string()))
        .col_expr(
            entities::outbox_job::Column::State,
            Expr::value(JOB_SUCCEEDED),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseOwner,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseExpiresAtUnix,
            Expr::value(Option::<i64>::None),
        )
        .col_expr(
            entities::outbox_job::Column::LastError,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            entities::outbox_job::Column::CompletedAtUnix,
            Expr::value(Some(now)),
        )
        .col_expr(
            entities::outbox_job::Column::UpdatedAtUnix,
            Expr::value(now),
        )
        .exec(conn)
        .await?;
    if completed.rows_affected != 1 {
        let job_is_missing = entities::outbox_job::Entity::find_by_id(job.id.clone())
            .one(conn)
            .await?
            .is_none();
        let repository_is_missing = entities::repository::Entity::find_by_id(job.repo_id.clone())
            .one(conn)
            .await?
            .is_none();
        if job_is_missing && repository_is_missing {
            return Ok(());
        }
        return Err(PostgresError::conflict(
            "outbox job lease was lost before completion",
        ));
    }
    prune_succeeded_outbox_jobs(conn, now).await?;
    Ok(())
}

async fn fail_outbox_job<C>(
    conn: &C,
    job: &ClaimedOutboxJob,
    worker_id: &str,
    error: String,
    now: i64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait + TransactionTrait<Transaction = DatabaseTransaction>,
{
    let attempts = next_retry_attempt(job.attempts)?;
    let terminal = is_terminal_retry_attempt(attempts);
    let next_run_at = if terminal {
        now
    } else {
        now.checked_add(retry_delay_seconds(attempts))
            .ok_or_else(|| PostgresError::internal_message("outbox retry time exceeds i64 range"))?
    };
    let completed_at_unix = terminal.then_some(now);
    let state = if terminal { JOB_FAILED } else { JOB_READY };
    let tx = conn.begin().await?;
    let persisted_error = truncate_error(error);
    let failed = entities::outbox_job::Entity::update_many()
        .filter(entities::outbox_job::Column::Id.eq(job.id.clone()))
        .filter(entities::outbox_job::Column::LeaseOwner.eq(worker_id.to_string()))
        .filter(entities::outbox_job::Column::CompletedAtUnix.is_null())
        .col_expr(entities::outbox_job::Column::State, Expr::value(state))
        .col_expr(
            entities::outbox_job::Column::Attempts,
            Expr::value(attempts),
        )
        .col_expr(
            entities::outbox_job::Column::NextRunAtUnix,
            Expr::value(next_run_at),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseOwner,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            entities::outbox_job::Column::LeaseExpiresAtUnix,
            Expr::value(Option::<i64>::None),
        )
        .col_expr(
            entities::outbox_job::Column::LastError,
            Expr::value(Some(persisted_error.clone())),
        )
        .col_expr(
            entities::outbox_job::Column::UpdatedAtUnix,
            Expr::value(now),
        )
        .col_expr(
            entities::outbox_job::Column::CompletedAtUnix,
            Expr::value(completed_at_unix),
        )
        .exec(&tx)
        .await?;
    if failed.rows_affected != 1 {
        return Err(PostgresError::conflict(
            "outbox job lease was lost before failure handling",
        ));
    }
    if terminal && job.kind == super::push_triggers::JOB_KIND {
        super::push_triggers::mark_terminal_failure(
            &tx,
            job,
            persisted_error,
            u64::try_from(now)
                .map_err(|_| PostgresError::internal_message("outbox failure time is negative"))?,
        )
        .await?;
    }
    tx.commit().await.map_err(PostgresError::internal)
}

async fn prune_succeeded_outbox_jobs<C>(conn: &C, now: i64) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let cutoff = now.checked_sub(SUCCEEDED_JOB_RETENTION_SECS).unwrap_or(0);
    entities::outbox_job::Entity::delete_many()
        .filter(entities::outbox_job::Column::State.eq(JOB_SUCCEEDED))
        .filter(entities::outbox_job::Column::CompletedAtUnix.lte(cutoff))
        .exec(conn)
        .await?;
    Ok(())
}

fn retry_delay_seconds(attempts: i64) -> i64 {
    let exponent =
        u32::try_from(attempts.checked_sub(1).unwrap_or_default().clamp(0, 6)).unwrap_or_default();
    5_i64
        .checked_mul(2_i64.pow(exponent))
        .unwrap_or(MAX_RETRY_DELAY_SECS)
        .min(MAX_RETRY_DELAY_SECS)
}

fn next_retry_attempt(previous_attempts: i64) -> Result<i64, PostgresError> {
    previous_attempts
        .checked_add(1)
        .filter(|attempts| *attempts >= 1)
        .ok_or_else(|| {
            PostgresError::internal_message("outbox attempt count is outside valid range")
        })
}

fn is_terminal_retry_attempt(attempts: i64) -> bool {
    attempts >= MAX_JOB_ATTEMPTS
}

fn truncate_error(error: String) -> String {
    const MAX_ERROR_CHARS: usize = 2_000;
    if error.chars().count() <= MAX_ERROR_CHARS {
        return error;
    }
    error.chars().take(MAX_ERROR_CHARS).collect()
}

fn unix_timestamp_i64(now_unix: u64) -> Result<i64, PostgresError> {
    i64::try_from(now_unix)
        .map_err(|_| PostgresError::internal_message("timestamp exceeds i64 range"))
}

#[cfg(test)]
mod tests;
