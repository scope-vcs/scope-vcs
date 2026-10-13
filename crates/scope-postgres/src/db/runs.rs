use super::integer_columns;
use super::{
    RunStore, entities,
    git_segments::insert_git_segment_references,
    native_runs::lock_native_runs_availability,
    object_references::insert_object_reference,
    run_attempt_persistence::{
        locked_attempt_context, locked_jobs, locked_run, save_attempt, save_jobs, save_run,
    },
};
use crate::error::PostgresError;
use scope_domain::{
    repository::RepositoryIncarnation,
    runs::{
        attempt::RunAttempt,
        job::{RunJob, create_run_jobs, request_run_cancellation, retry_run},
        run::Run,
        step::{AttemptConclusion, RunAttemptStep},
        workflow::revision::WorkflowRevision,
    },
};
use sea_orm::{
    ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, ExprTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect, TransactionTrait, TryInsertResult, sea_query::OnConflict,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchClaim {
    pub repository: RepositoryIncarnation,
    pub run: Run,
    pub job: RunJob,
    pub attempt: RunAttempt,
    pub steps: Vec<RunAttemptStep>,
    pub workflow_revision: WorkflowRevision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptMutation {
    pub claim: DispatchClaim,
    pub transitioned: bool,
}

impl AttemptMutation {
    pub fn transition(&self) -> Option<&DispatchClaim> {
        self.transitioned.then_some(&self.claim)
    }
}

impl RunStore {
    #[cfg(any(test, feature = "seeding"))]
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "enqueue_run"))]
    pub async fn enqueue_run(
        &self,
        run: Run,
        revision: WorkflowRevision,
    ) -> Result<EnqueueRunResult, PostgresError> {
        let tx = self.db.begin().await?;
        let stored = enqueue_run_in_transaction(&tx, run, revision).await?;
        tx.commit().await?;
        Ok(stored)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "heartbeat_attempt"))]
    pub async fn heartbeat_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
    ) -> Result<(RunAttempt, bool), PostgresError> {
        let tx = self.db.begin().await?;
        let (run, job, mut attempt) = locked_attempt_context(&tx, attempt_id).await?;
        let cancellation_requested = attempt
            .heartbeat(&run, &job, token_hash, now_unix, lease_expires_at_unix)
            .map_err(PostgresError::from)?;
        save_attempt(&tx, &attempt).await?;
        tx.commit().await?;
        Ok((attempt, cancellation_requested))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "complete_attempt"))]
    pub async fn complete_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        conclusion: AttemptConclusion,
        logs_truncated: bool,
        now_unix: u64,
    ) -> Result<AttemptMutation, PostgresError> {
        self.mutate_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.complete(
                run,
                job,
                steps,
                token_hash,
                conclusion,
                logs_truncated,
                now_unix,
            )
        })
        .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "reject_capacity_attempt"))]
    pub async fn reject_capacity_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        message: &scope_domain::runs::step::SetupFailureMessage,
        now_unix: u64,
    ) -> Result<AttemptMutation, PostgresError> {
        self.mutate_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.reject_capacity(run, job, steps, token_hash, message, now_unix)
        })
        .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "abandon_attempt"))]
    pub async fn abandon_attempt(
        &self,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<AttemptMutation, PostgresError> {
        self.mutate_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.abandon(run, job, steps, token_hash, now_unix)
        })
        .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "confirm_provider_cancellation"))]
    pub async fn confirm_provider_cancellation(
        &self,
        attempt_id: &str,
        now_unix: u64,
    ) -> Result<AttemptMutation, PostgresError> {
        self.mutate_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.confirm_provider_cancellation(run, job, steps, now_unix)
        })
        .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "expire_attempt"))]
    pub async fn expire_attempt(
        &self,
        attempt_id: &str,
        now_unix: u64,
    ) -> Result<AttemptMutation, PostgresError> {
        self.mutate_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.expire(run, job, steps, now_unix)
        })
        .await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "expired_attempt_ids"))]
    pub async fn expired_attempt_ids(
        &self,
        now_unix: u64,
        limit: u64,
    ) -> Result<Vec<String>, PostgresError> {
        let now_unix = integer_columns::u64_to_i64(now_unix, "attempt recovery time")?;
        let maximum_age_cutoff = now_unix
            .saturating_sub(scope_domain::runs::attempt::MAX_RUN_ATTEMPT_AGE_SECONDS as i64);
        Ok(entities::run_attempt::Entity::find()
            .filter(
                entities::run_attempt::Column::State
                    .is_in(["dispatching".to_string(), "running".to_string()]),
            )
            .filter(
                entities::run_attempt::Column::LeaseExpiresAtUnix
                    .lte(now_unix)
                    .or(entities::run_attempt::Column::CreatedAtUnix.lte(maximum_age_cutoff)),
            )
            .order_by_asc(entities::run_attempt::Column::LeaseExpiresAtUnix)
            .order_by_asc(entities::run_attempt::Column::Id)
            .limit(limit)
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(|attempt| attempt.id)
            .collect())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_run_cancellation"))]
    pub async fn request_run_cancellation(
        &self,
        actor_user_id: &str,
        repository_id: &str,
        run_id: &str,
        now_unix: u64,
    ) -> Result<Run, PostgresError> {
        let tx = self.db.begin().await?;
        authorize_run_control(&tx, actor_user_id, repository_id).await?;
        let active_auto_merge =
            super::request_auto_merge::lock_active_auto_merge_for_run(&tx, run_id).await?;
        let mut jobs = locked_jobs(&tx, run_id).await?;
        let mut run = locked_run(&tx, run_id).await?;
        require_run_repository(&run, repository_id)?;
        request_run_cancellation(&mut run, &mut jobs, now_unix).map_err(PostgresError::from)?;
        save_jobs(&tx, &jobs).await?;
        save_run(&tx, &run).await?;
        super::request_auto_merge::stop_auto_merge_for_terminal_run(&tx, active_auto_merge, &run)
            .await?;
        tx.commit().await?;
        Ok(run)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "retry_run"))]
    pub async fn retry_run(
        &self,
        actor_user_id: &str,
        repository_id: &str,
        run_id: &str,
        now_unix: u64,
    ) -> Result<Run, PostgresError> {
        let tx = self.db.begin().await?;
        authorize_run_control(&tx, actor_user_id, repository_id).await?;
        let _active_auto_merge =
            super::request_auto_merge::lock_active_auto_merge_for_run(&tx, run_id).await?;
        lock_native_runs_availability(&tx, repository_id)
            .await?
            .require()?;
        let mut jobs = locked_jobs(&tx, run_id).await?;
        let mut run = locked_run(&tx, run_id).await?;
        require_run_repository(&run, repository_id)?;
        let revision = workflow_revision_for_run(&tx, &run).await?;
        retry_run(&mut run, &mut jobs, &revision, now_unix).map_err(PostgresError::from)?;
        save_jobs(&tx, &jobs).await?;
        save_run(&tx, &run).await?;
        tx.commit().await?;
        Ok(run)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "run"))]
    pub async fn run(&self, run_id: &str) -> Result<Option<Run>, PostgresError> {
        entities::run::Entity::find_by_id(run_id.to_string())
            .one(self.db.as_ref())
            .await?
            .map(entities::run::Model::try_into_domain)
            .transpose()
    }
}

async fn authorize_run_control(
    tx: &DatabaseTransaction,
    actor_user_id: &str,
    repository_id: &str,
) -> Result<(), PostgresError> {
    super::acquire_aggregate_lock(tx, "repository", repository_id).await?;
    let access =
        super::repository_access::repository_access(tx, repository_id, Some(actor_user_id))
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
    access.ensure_member()?;
    access.ensure_run_reader()?;
    Ok(())
}

fn require_run_repository(run: &Run, repository_id: &str) -> Result<(), PostgresError> {
    if !run.belongs_to_repository(repository_id) {
        return Err(PostgresError::not_found("run not found"));
    }
    Ok(())
}

pub struct EnqueueRunResult {
    pub run: Run,
    pub inserted: bool,
}

pub(super) async fn enqueue_run_in_transaction(
    tx: &DatabaseTransaction,
    run: Run,
    revision: WorkflowRevision,
) -> Result<EnqueueRunResult, PostgresError> {
    run.validate_workflow_revision(&revision)
        .map_err(PostgresError::from)?;
    lock_native_runs_availability(tx, run.workflow.repository_id())
        .await?
        .require()?;
    let requested_jobs = create_run_jobs(&run, &revision).map_err(PostgresError::from)?;
    save_workflow_revision(tx, &revision, run.created_at_unix).await?;
    let model = entities::run::ActiveModel::from_domain(&run)?;
    let result = entities::run::Entity::insert(model)
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .try_insert()
        .exec(tx)
        .await?;
    let inserted = matches!(result, TryInsertResult::Inserted(_));
    if !inserted {
        let stored = entities::run::Entity::find()
            .filter(entities::run::Column::RepoId.eq(run.workflow.repository_id().to_string()))
            .filter(entities::run::Column::IdempotencyKey.eq(run.idempotency_key.clone()))
            .one(tx)
            .await?
            .ok_or_else(|| {
                PostgresError::conflict("run id is already used by another idempotency key")
            })?
            .try_into_domain()?;
        if !stored.has_same_enqueue_request_identity(&run) {
            return Err(PostgresError::conflict(
                "run idempotency key is already used by a different enqueue request",
            ));
        }
        return Ok(EnqueueRunResult {
            run: stored,
            inserted: false,
        });
    }
    entities::run_job::Entity::insert_many(
        requested_jobs
            .iter()
            .map(entities::run_job::Model::from_domain)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(IntoActiveModel::into_active_model),
    )
    .exec(tx)
    .await?;
    for object in run.source.retained_objects() {
        insert_object_reference(tx, "run_source", &run.id, object).await?;
    }
    insert_git_segment_references(
        tx,
        "run_source",
        &run.id,
        run.source.retained_git_segments(),
    )
    .await?;
    Ok(EnqueueRunResult {
        run,
        inserted: true,
    })
}

pub(super) async fn save_workflow_revision(
    tx: &DatabaseTransaction,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), PostgresError> {
    entities::workflow_revision::Entity::insert(
        entities::workflow_revision::Model::from_domain(revision, created_at_unix)?
            .into_active_model(),
    )
    .on_conflict(
        OnConflict::column(entities::workflow_revision::Column::Digest)
            .do_nothing()
            .to_owned(),
    )
    .try_insert()
    .exec(tx)
    .await?;
    let persisted = entities::workflow_revision::Entity::find_by_id(revision.digest().to_string())
        .one(tx)
        .await?
        .ok_or_else(|| PostgresError::internal_message("workflow revision was not stored"))?;
    let persisted = persisted.try_into_domain(revision.workflow().clone())?;
    if &persisted != revision {
        return Err(PostgresError::conflict(
            "workflow revision digest is already used by a different definition",
        ));
    }
    Ok(())
}

pub(super) async fn workflow_revision_for_run(
    tx: &DatabaseTransaction,
    run: &Run,
) -> Result<WorkflowRevision, PostgresError> {
    entities::workflow_revision::Entity::find_by_id(run.workflow_revision_digest.clone())
        .one(tx)
        .await?
        .ok_or_else(|| PostgresError::internal_message("run workflow revision is missing"))?
        .try_into_domain(run.workflow.clone())
}

pub(super) fn unique_conflict(error: DbErr, message: &str) -> PostgresError {
    if matches!(
        error.sql_err(),
        Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
    ) {
        PostgresError::conflict(message)
    } else {
        PostgresError::internal(error)
    }
}
