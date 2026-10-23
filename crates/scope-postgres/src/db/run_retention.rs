use super::integer_columns;
use super::{
    RunStore, acquire_aggregate_lock,
    cleanup_queue::queue::queue_pending_source_blob_deletion_rows, entities,
    locks::acquire_shared_aggregate_lock, request_auto_merge::request_id_for_check_run,
};
use crate::error::PostgresError;
use scope_domain::{content::SourceBlob, runs::run::RunState};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    TransactionTrait,
    sea_query::{Expr, Query},
};
use std::collections::BTreeSet;

fn prunable(cutoff: i64) -> Condition {
    let terminal = RunState::ALL
        .into_iter()
        .filter(|state| state.is_terminal())
        .map(RunState::as_str);
    Condition::all()
        .add(entities::run::Column::State.is_in(terminal))
        .add(entities::run::Column::CompletedAtUnix.lte(cutoff))
        .add(active_auto_merge_evidence_is_absent())
}

impl RunStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "prune_terminal_runs"))]
    pub async fn prune_terminal_runs(
        &self,
        completed_before_unix: u64,
        now_unix: u64,
        limit: u64,
    ) -> Result<usize, PostgresError> {
        let cutoff = integer_columns::u64_to_i64(completed_before_unix, "run retention cutoff")?;
        let tx = self.db.begin().await?;
        let candidates = entities::run::Entity::find()
            .filter(prunable(cutoff))
            .order_by_asc(entities::run::Column::CompletedAtUnix)
            .order_by_asc(entities::run::Column::Id)
            .limit(limit)
            .all(&tx)
            .await?;
        let candidate_ids = candidates.into_iter().map(|run| run.id).collect::<Vec<_>>();
        if candidate_ids.is_empty() {
            tx.commit().await?;
            return Ok(0);
        }

        let mut request_ids = Vec::new();
        for run_id in &candidate_ids {
            if let Some(request_id) = request_id_for_check_run(&tx, run_id).await? {
                request_ids.push(request_id);
            }
        }
        request_ids.sort_unstable();
        request_ids.dedup();
        for request_id in request_ids {
            acquire_aggregate_lock(&tx, "request", &request_id).await?;
        }
        lock_run_evidence_for_pruning(&tx).await?;

        let mut lock_ids = candidate_ids.clone();
        lock_ids.sort_unstable();
        for run_id in lock_ids {
            entities::run::Entity::find_by_id(run_id)
                .lock_exclusive()
                .one(&tx)
                .await?;
        }

        let models = entities::run::Entity::find()
            .filter(entities::run::Column::Id.is_in(candidate_ids))
            .filter(prunable(cutoff))
            .all(&tx)
            .await?;
        let runs = models
            .into_iter()
            .map(entities::run::Model::try_into_domain)
            .collect::<Result<Vec<_>, _>>()?;
        let run_ids = runs.iter().map(|run| run.id.clone()).collect::<Vec<_>>();
        if run_ids.is_empty() {
            tx.commit().await?;
            return Ok(0);
        }
        let sources = runs
            .iter()
            .flat_map(|run| run.source.retained_objects())
            .cloned()
            .collect::<Vec<SourceBlob>>();
        let workflow_digests = runs
            .iter()
            .map(|run| run.workflow_revision_digest.clone())
            .collect::<BTreeSet<_>>();

        entities::run_log::Entity::delete_many()
            .filter(entities::run_log::Column::RunId.is_in(run_ids.clone()))
            .exec(&tx)
            .await?;
        let attempt_ids = entities::run_attempt::Entity::find()
            .select_only()
            .column(entities::run_attempt::Column::Id)
            .filter(entities::run_attempt::Column::RunId.is_in(run_ids.clone()))
            .into_tuple::<String>()
            .all(&tx)
            .await?;
        entities::run_attempt_step::Entity::delete_many()
            .filter(entities::run_attempt_step::Column::AttemptId.is_in(attempt_ids))
            .exec(&tx)
            .await?;
        entities::run_attempt::Entity::delete_many()
            .filter(entities::run_attempt::Column::RunId.is_in(run_ids.clone()))
            .exec(&tx)
            .await?;
        delete_run_source_references(&tx, &run_ids).await?;
        entities::run::Entity::delete_many()
            .filter(entities::run::Column::Id.is_in(run_ids))
            .exec(&tx)
            .await?;
        delete_orphaned_workflow_revisions(&tx, workflow_digests).await?;
        let queued_blob_count = sources.len();
        queue_pending_source_blob_deletion_rows(&tx, sources, now_unix).await?;
        tx.commit().await?;

        Ok(queued_blob_count)
    }
}

const RUN_EVIDENCE_RETENTION: (&str, &str) = ("run-retention", "request-check-evidence");

async fn lock_run_evidence_for_pruning<C: ConnectionTrait>(conn: &C) -> Result<(), PostgresError> {
    let (namespace, id) = RUN_EVIDENCE_RETENTION;
    acquire_aggregate_lock(conn, namespace, id).await
}

pub(super) async fn lock_run_evidence_for_recording<C: ConnectionTrait>(
    conn: &C,
) -> Result<(), PostgresError> {
    let (namespace, id) = RUN_EVIDENCE_RETENTION;
    acquire_shared_aggregate_lock(conn, namespace, id).await
}

fn active_auto_merge_evidence_is_absent() -> sea_orm::sea_query::SimpleExpr {
    Expr::cust(
        "NOT EXISTS (
            SELECT 1
              FROM scope_request_auto_merge_intents intent
              JOIN scope_requests request
                ON request.id = intent.request_id AND request.head_oid = intent.head_oid
              JOIN scope_request_check_evaluations evaluation
                ON evaluation.id = request.current_check_evaluation_id
             WHERE intent.status = 'Active'
               AND evaluation.checks @> jsonb_build_array(
                   jsonb_build_object('provider', 'native', 'run_id', scope_runs.id)
               )
        )",
    )
}

pub(super) async fn delete_run_source_references<C: ConnectionTrait>(
    conn: &C,
    run_ids: &[String],
) -> Result<(), PostgresError> {
    if run_ids.is_empty() {
        return Ok(());
    }
    entities::object_reference::Entity::delete_many()
        .filter(entities::object_reference::Column::RefKind.eq("run_source"))
        .filter(entities::object_reference::Column::RefId.is_in(run_ids.to_vec()))
        .exec(conn)
        .await?;
    Ok(())
}

pub(super) async fn delete_orphaned_workflow_revisions<C: ConnectionTrait>(
    conn: &C,
    digests: BTreeSet<String>,
) -> Result<(), PostgresError> {
    if digests.is_empty() {
        return Ok(());
    }
    entities::workflow_revision::Entity::delete_many()
        .filter(entities::workflow_revision::Column::Digest.is_in(digests))
        .filter(
            entities::workflow_revision::Column::Digest.not_in_subquery(
                Query::select()
                    .column(entities::run::Column::WorkflowRevisionDigest)
                    .from(entities::run::Entity)
                    .to_owned(),
            ),
        )
        .exec(conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{lock_run_evidence_for_pruning, lock_run_evidence_for_recording};
    use crate::db::{
        MetadataStore, TestDatabaseTarget,
        locks::{backend_pid, wait_for_transaction_waiter},
    };
    use sea_orm::TransactionTrait;
    use std::time::Duration;

    #[tokio::test]
    async fn check_recorders_share_run_evidence_while_pruning_waits_for_them() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        let seeded = store.db.begin().await.unwrap();
        lock_run_evidence_for_pruning(&seeded).await.unwrap();
        seeded.commit().await.unwrap();
        let first = store.db.begin().await.unwrap();
        lock_run_evidence_for_recording(&first).await.unwrap();
        let second = store.db.begin().await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(5),
            lock_run_evidence_for_recording(&second),
        )
        .await
        .expect("a second check recorder waited for the first")
        .unwrap();

        let first_pid = backend_pid(&first).await;
        let pruning_store = store.clone();
        let pruning = tokio::spawn(async move {
            let tx = pruning_store.db.begin().await.unwrap();
            lock_run_evidence_for_pruning(&tx).await.unwrap();
            tx.commit().await.unwrap();
        });
        wait_for_transaction_waiter(&store, first_pid).await;
        first.commit().await.unwrap();
        second.commit().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), pruning)
            .await
            .expect("pruning proceeds once recorders finish")
            .unwrap();
    }
}
