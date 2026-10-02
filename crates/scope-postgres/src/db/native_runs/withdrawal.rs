//! Settles the repositories of an account that lost native runs. Each request
//! and run settles in its own transaction, taking the same locks in the same
//! order as the operations it competes with, and is skipped if the owner was
//! listed again in the meantime.

use super::{availability, owned_repositories};
use crate::{
    db::{
        acquire_aggregate_lock, entities,
        request_auto_merge::{lock_active_auto_merge_for_run, lock_active_intent_for_request},
        request_checks::{evaluation_for_head, save_evaluation, stop_auto_merge_for_evaluation},
        request_rows::request_by_id,
        run_attempt_persistence::{locked_jobs, locked_run, save_jobs, save_run},
        run_state_sql::run_active_states,
    },
    error::PostgresError,
};
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::RequestCheckEvaluation,
    runs::{job::request_run_cancellation, run::Run},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait, QueryFilter,
    Statement, TransactionTrait,
};

/// What removing an account changed, for the caller to publish.
#[derive(Clone, Debug, Default)]
pub struct NativeRunsWithdrawal {
    pub user_id: String,
    /// Whether this removal unlisted the account; `false` when it was not listed.
    pub removed: bool,
    /// The account's repositories, whose availability changed.
    pub repositories: Vec<RepositoryIncarnation>,
    pub withdrawn_evaluations: Vec<RequestCheckEvaluation>,
    pub canceled_runs: Vec<Run>,
}

pub(super) async fn settle(
    db: &DatabaseConnection,
    user_id: &str,
    removed: bool,
    now_unix: u64,
) -> Result<NativeRunsWithdrawal, PostgresError> {
    let mut withdrawal = NativeRunsWithdrawal {
        user_id: user_id.to_string(),
        removed,
        ..Default::default()
    };
    // Withdraw evaluations while their runs are still unfinished; once
    // canceled, those runs would read as failures rather than a wait.
    for request_id in waiting_request_ids(db, user_id).await? {
        if let Some(evaluation) = withdraw_evaluation(db, &request_id, now_unix).await? {
            withdrawal.withdrawn_evaluations.push(evaluation);
        }
    }
    for run_id in unfinished_run_ids(db, user_id).await? {
        if let Some(run) = cancel_run(db, &run_id, now_unix).await? {
            withdrawal.canceled_runs.push(run);
        }
    }
    withdrawal.repositories = owned_repositories(db, user_id).await?;
    Ok(withdrawal)
}

async fn waiting_request_ids(
    db: &DatabaseConnection,
    user_id: &str,
) -> Result<Vec<String>, PostgresError> {
    ids(
        db,
        "SELECT request.id
         FROM scope_requests request
         JOIN scope_repositories repo ON repo.id = request.repo_id
         JOIN scope_request_check_evaluations evaluation
           ON evaluation.request_id = request.id AND evaluation.head_oid = request.head_oid
         WHERE repo.owner_user_id = $1
           AND request.closed_at_unix IS NULL AND request.merged_at_unix IS NULL
           AND evaluation.state IN ('awaiting-approval', 'started')
         ORDER BY request.id"
            .to_string(),
        user_id,
    )
    .await
}

async fn unfinished_run_ids(
    db: &DatabaseConnection,
    user_id: &str,
) -> Result<Vec<String>, PostgresError> {
    ids(
        db,
        format!(
            "SELECT run.id
             FROM scope_runs run
             JOIN scope_repositories repo ON repo.id = run.repo_id
             WHERE repo.owner_user_id = $1 AND run.state IN ({})
             ORDER BY run.id",
            run_active_states()
        ),
        user_id,
    )
    .await
}

async fn ids(
    db: &DatabaseConnection,
    sql: String,
    user_id: &str,
) -> Result<Vec<String>, PostgresError> {
    db.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        sql,
        [user_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)?
    .into_iter()
    .map(|row| {
        row.try_get::<String>("", "id")
            .map_err(PostgresError::internal)
    })
    .collect()
}

async fn withdraw_evaluation(
    db: &DatabaseConnection,
    request_id: &str,
    now_unix: u64,
) -> Result<Option<RequestCheckEvaluation>, PostgresError> {
    let tx = db.begin().await.map_err(PostgresError::internal)?;
    acquire_aggregate_lock(&tx, "request", request_id).await?;
    let active_auto_merge = lock_active_intent_for_request(&tx, request_id).await?;
    let Some(request) = request_by_id(&tx, request_id).await? else {
        return Ok(None);
    };
    if request.is_terminal()
        || availability(&tx, &request.repo_id, "")
            .await?
            .is_available()
    {
        return Ok(None);
    }
    let Some(evaluation) = evaluation_for_head(&tx, &request.id, &request.head_oid).await? else {
        return Ok(None);
    };
    let run_ids = evaluation.run_ids().map(str::to_string).collect::<Vec<_>>();
    let run_states = entities::run::Entity::find()
        .filter(entities::run::Column::Id.is_in(run_ids))
        .all(&tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| row.try_into_domain().map(|run| (run.id, run.state)))
        .collect::<Result<Vec<_>, _>>()?;
    let Some(withdrawn) = evaluation.withdraw_native_runs(&run_states, now_unix)? else {
        return Ok(None);
    };
    save_evaluation(&tx, &withdrawn).await?;
    stop_auto_merge_for_evaluation(&tx, active_auto_merge, &request, &withdrawn).await?;
    tx.commit().await.map_err(PostgresError::internal)?;
    Ok(Some(withdrawn))
}

/// Cancels like a maintainer would: queued and blocked jobs end at once and a
/// running attempt is told to stop at its next heartbeat.
async fn cancel_run(
    db: &DatabaseConnection,
    run_id: &str,
    now_unix: u64,
) -> Result<Option<Run>, PostgresError> {
    let tx = db.begin().await.map_err(PostgresError::internal)?;
    let active_auto_merge = lock_active_auto_merge_for_run(&tx, run_id).await?;
    let mut jobs = locked_jobs(&tx, run_id).await?;
    let mut run = locked_run(&tx, run_id).await?;
    if availability(&tx, run.workflow.repository_id(), "")
        .await?
        .is_available()
        || !request_run_cancellation(&mut run, &mut jobs, now_unix)?
    {
        return Ok(None);
    }
    save_jobs(&tx, &jobs).await?;
    save_run(&tx, &run).await?;
    crate::db::request_auto_merge::stop_auto_merge_for_terminal_run(&tx, active_auto_merge, &run)
        .await?;
    tx.commit().await.map_err(PostgresError::internal)?;
    Ok(Some(run))
}
