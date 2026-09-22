//! The checks recorded for request heads, and the transactions that start them.

use super::{
    RequestStore, entities,
    request_access::{ensure_user_exists, lock_request_repository},
    runs::{enqueue_run_in_transaction, save_workflow_revision},
};
use crate::error::PostgresError;
use scope_domain::{
    requests::{
        RequestCheckEvaluation, RequestCheckPlan, RequestRevision,
        stop_request_auto_merge_for_check_evaluation,
    },
    runs::{
        run::Run,
        workflow::{identity::WorkflowIdentity, revision::WorkflowRevision},
    },
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseTransaction, EntityTrait, IntoActiveModel,
    QueryFilter, TransactionTrait, sea_query::OnConflict,
};

/// A head's evaluation together with the runs it starts now and the revisions
/// it may start later.
#[derive(Clone, Debug)]
pub struct RecordRequestChecksCommand {
    pub evaluation: RequestCheckEvaluation,
    pub revisions: Vec<WorkflowRevision>,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug)]
pub struct ApproveRequestChecksCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct RequestChecksMutation {
    pub evaluation: RequestCheckEvaluation,
    /// Runs this transaction created; a repeated evaluation creates none.
    pub created_runs: Vec<Run>,
}

impl RequestStore {
    pub async fn record_request_checks(
        &self,
        command: RecordRequestChecksCommand,
    ) -> Result<RequestChecksMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        super::acquire_aggregate_lock(&tx, "request", &command.evaluation.request_id).await?;
        super::run_retention::lock_run_evidence_retention(&tx).await?;
        let active_auto_merge = super::request_auto_merge::lock_active_intent_for_request(
            &tx,
            &command.evaluation.request_id,
        )
        .await?;
        // Evaluating reads the head outside this lock, so the request may have been
        // closed or merged since. A request that can no longer merge starts nothing.
        let request = super::request_rows::request_by_id(&tx, &command.evaluation.request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        if request.is_terminal() {
            return Err(PostgresError::conflict(
                "request can no longer merge, so its checks are not evaluated",
            ));
        }
        // The first evaluation of a head stands. A later one, from someone looking at
        // a request while its push was still evaluating, must not undo an approval.
        if let Some(evaluation) = evaluation_for_head(
            &tx,
            &command.evaluation.request_id,
            &command.evaluation.head_oid,
        )
        .await?
        {
            return Ok(RequestChecksMutation {
                evaluation,
                created_runs: Vec::new(),
            });
        }
        let created_runs = start_runs(&tx, &command.revisions, command.runs).await?;
        for revision in &command.revisions {
            save_workflow_revision(&tx, revision, command.evaluation.updated_at_unix).await?;
        }
        save_evaluation(&tx, &command.evaluation).await?;
        if let Some(stored) = active_auto_merge
            && let Some(stopped) = stop_request_auto_merge_for_check_evaluation(
                &request,
                &stored.intent,
                &command.evaluation,
                super::request_auto_merge::automatic_event_id("stopped", &stored.intent.id),
            )?
        {
            super::request_auto_merge::persist_existing_auto_merge_mutation(
                &tx,
                stored.model,
                &stopped,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RequestChecksMutation {
            evaluation: command.evaluation,
            created_runs,
        })
    }

    /// A maintainer starts the checks recorded for the request's current head.
    pub async fn approve_request_checks(
        &self,
        command: ApproveRequestChecksCommand,
    ) -> Result<RequestChecksMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        super::run_retention::lock_run_evidence_retention(&tx).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        if !repo.access.is_maintainer() {
            return Err(PostgresError::permission_denied("repo maintainer required"));
        }
        let evaluation = evaluation_for_head(&tx, &request.id, &request.head_oid)
            .await?
            .ok_or_else(|| PostgresError::not_found("request head has no recorded checks"))?;
        evaluation.ensure_awaiting_approval()?;
        let mut revisions = Vec::with_capacity(evaluation.checks.len());
        for check in &evaluation.checks {
            let identity = WorkflowIdentity::new(
                &request.repo_id,
                scope_domain::runs::workflow::identity::WorkflowPath::parse(
                    check.workflow_path.clone(),
                )
                .map_err(PostgresError::invalid_input)?,
            )
            .map_err(PostgresError::invalid_input)?;
            let revision = entities::workflow_revision::Entity::find_by_id(
                check.workflow_revision_digest.clone(),
            )
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("recorded check revision is missing"))?
            .try_into_domain(identity)?;
            revisions.push(revision);
        }
        let RequestCheckPlan { evaluation, runs } = RequestCheckPlan::approve(
            &request,
            evaluation,
            &revisions,
            &command.actor_user_id,
            command.now_unix,
        )?;
        let created_runs = start_runs(&tx, &revisions, runs).await?;
        save_evaluation(&tx, &evaluation).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RequestChecksMutation {
            evaluation,
            created_runs,
        })
    }

    /// The revision the request's most recent push saved.
    pub async fn latest_request_revision(
        &self,
        request_id: &str,
    ) -> Result<Option<RequestRevision>, PostgresError> {
        super::request_revision_rows::latest_revision_for_request(self.db.as_ref(), request_id)
            .await
    }

    pub async fn request_check_evaluation(
        &self,
        request_id: &str,
        head_oid: &str,
    ) -> Result<Option<RequestCheckEvaluation>, PostgresError> {
        evaluation_for_head(self.db.as_ref(), request_id, head_oid).await
    }

    /// The evaluation for each `(request id, head oid)` pair that has one.
    pub async fn request_check_evaluations(
        &self,
        heads: &[(String, String)],
    ) -> Result<Vec<RequestCheckEvaluation>, PostgresError> {
        if heads.is_empty() {
            return Ok(Vec::new());
        }
        entities::request_check_evaluation::Entity::find()
            .filter(head_pairs_condition(heads))
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(entities::request_check_evaluation::Model::try_into_domain)
            .collect()
    }
}

fn head_pairs_condition(heads: &[(String, String)]) -> Condition {
    heads
        .iter()
        .fold(Condition::any(), |condition, (request_id, head_oid)| {
            condition.add(
                Condition::all()
                    .add(
                        entities::request_check_evaluation::Column::RequestId
                            .eq(request_id.clone()),
                    )
                    .add(entities::request_check_evaluation::Column::HeadOid.eq(head_oid.clone())),
            )
        })
}

async fn start_runs(
    tx: &DatabaseTransaction,
    revisions: &[WorkflowRevision],
    runs: Vec<Run>,
) -> Result<Vec<Run>, PostgresError> {
    let mut created = Vec::new();
    for run in runs {
        let revision = revisions
            .iter()
            .find(|revision| revision.digest() == run.workflow_revision_digest)
            .cloned()
            .ok_or_else(|| {
                PostgresError::invalid_input("request check run has no workflow revision")
            })?;
        let stored = enqueue_run_in_transaction(tx, run, revision).await?;
        if stored.inserted {
            created.push(stored.run);
        }
    }
    Ok(created)
}

async fn evaluation_for_head<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
    head_oid: &str,
) -> Result<Option<RequestCheckEvaluation>, PostgresError> {
    entities::request_check_evaluation::Entity::find_by_id((
        request_id.to_string(),
        head_oid.to_string(),
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    .map(entities::request_check_evaluation::Model::try_into_domain)
    .transpose()
}

async fn save_evaluation(
    tx: &DatabaseTransaction,
    evaluation: &RequestCheckEvaluation,
) -> Result<(), PostgresError> {
    let model = entities::request_check_evaluation::Model::from_domain(evaluation)?;
    entities::request_check_evaluation::Entity::insert(model.into_active_model())
        .on_conflict(
            OnConflict::columns([
                entities::request_check_evaluation::Column::RequestId,
                entities::request_check_evaluation::Column::HeadOid,
            ])
            .update_columns([
                entities::request_check_evaluation::Column::State,
                entities::request_check_evaluation::Column::Message,
                entities::request_check_evaluation::Column::Checks,
                entities::request_check_evaluation::Column::UpdatedAtUnix,
            ])
            .to_owned(),
        )
        .exec(tx)
        .await
        .map_err(PostgresError::internal)?;
    Ok(())
}

#[cfg(test)]
mod tests;
