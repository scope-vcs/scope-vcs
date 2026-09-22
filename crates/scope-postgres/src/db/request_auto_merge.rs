//! Durable request auto-merge authorization and reconciliation leases.

use super::{
    GeneratedIdKind, GeneratedIdSource, RequestStore, acquire_aggregate_lock, entities,
    generated_ids::generate_id,
    integer_columns::u64_to_i64,
    request_access::{ensure_user_exists, lock_request_repository},
    request_revision_rows::latest_revision_for_request,
    request_rows::{insert_request_event_row, request_by_id, save_request_row},
};
use crate::error::PostgresError;
use scope_domain::{
    requests::{
        AuthorizeRequestAutoMergeInput, CancelRequestAutoMergeInput, RequestAutoMergeIntent,
        RequestAutoMergeMutation, RequestAutoMergeStopReason, authorize_request_auto_merge,
        cancel_request_auto_merge, stop_request_auto_merge, stop_request_auto_merge_for_check_run,
    },
    runs::run::RunState,
};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Statement, TransactionTrait,
    sea_query::{LockBehavior, LockType},
};

#[derive(Clone, Debug)]
pub struct AuthorizeRequestAutoMergeCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub expected_revision_id: String,
    pub expected_head_oid: String,
    pub intent_id: String,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct CancelRequestAutoMergeCommand {
    pub request_id: String,
    pub actor_user_id: String,
    pub expected_intent_id: String,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct ClaimDueRequestAutoMergesCommand {
    pub now_unix: u64,
    pub lease_expires_at_unix: u64,
    pub limit: u64,
}

#[derive(Clone, Debug)]
pub struct ClaimedRequestAutoMerge {
    pub intent: RequestAutoMergeIntent,
    pub claim_token: String,
    pub claim_expires_at_unix: u64,
    pub attempt: u32,
    pub owner: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct ReleaseRequestAutoMergeClaimCommand {
    pub intent_id: String,
    pub claim_token: String,
    pub next_attempt_at_unix: u64,
    pub last_error: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct StopClaimedRequestAutoMergeCommand {
    pub intent_id: String,
    pub claim_token: String,
    pub reason: RequestAutoMergeStopReason,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct RequestAutoMergeCheckState {
    pub evaluation: Option<scope_domain::requests::RequestCheckEvaluation>,
    pub run_states: Vec<(String, RunState)>,
}

impl RequestStore {
    pub async fn authorize_request_auto_merge(
        &self,
        command: AuthorizeRequestAutoMergeCommand,
    ) -> Result<RequestAutoMergeMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let revision = latest_revision_for_request(&tx, &request.id)
            .await?
            .ok_or_else(|| PostgresError::conflict("request has no recorded revision"))?;
        let current = lock_active_intent_for_request(&tx, &request.id).await?;
        let mutation = authorize_request_auto_merge(
            &request,
            &revision,
            current.as_ref().map(|row| &row.intent),
            AuthorizeRequestAutoMergeInput {
                id: command.intent_id,
                repo_id: request.repo_id.clone(),
                repository_incarnation_id: repo.record.incarnation_id.clone(),
                request_id: command.request_id,
                actor_user_id: command.actor_user_id,
                actor_is_maintainer: repo.access.is_maintainer(),
                expected_revision_id: command.expected_revision_id,
                expected_head_oid: command.expected_head_oid,
                event_id: command.event_id,
                now_unix: command.now_unix,
            },
        )?;
        lock_request_check_evidence(&tx, &request.id, &revision.new_head_oid).await?;
        persist_new_auto_merge_mutation(&tx, &mutation).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn cancel_request_auto_merge(
        &self,
        command: CancelRequestAutoMergeCommand,
    ) -> Result<RequestAutoMergeMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let current = lock_active_intent_for_request(&tx, &request.id)
            .await?
            .ok_or_else(|| PostgresError::conflict("auto-merge is not active"))?;
        let mutation = cancel_request_auto_merge(
            &request,
            &current.intent,
            CancelRequestAutoMergeInput {
                request_id: command.request_id,
                actor_user_id: command.actor_user_id,
                actor_is_maintainer: repo.access.is_maintainer(),
                expected_intent_id: command.expected_intent_id,
                event_id: command.event_id,
                now_unix: command.now_unix,
            },
        )?;
        persist_existing_auto_merge_mutation(&tx, current.model, &mutation).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }

    pub async fn request_auto_merge_intent(
        &self,
        request_id: &str,
    ) -> Result<Option<RequestAutoMergeIntent>, PostgresError> {
        entities::request_auto_merge_intent::Entity::find()
            .filter(entities::request_auto_merge_intent::Column::RequestId.eq(request_id))
            .order_by_desc(entities::request_auto_merge_intent::Column::CreatedPosition)
            .order_by_desc(entities::request_auto_merge_intent::Column::CreatedAtUnix)
            .order_by_desc(entities::request_auto_merge_intent::Column::Id)
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .map(|row| row.try_into_domain())
            .transpose()
    }

    pub async fn request_auto_merge_check_state(
        &self,
        intent: &RequestAutoMergeIntent,
    ) -> Result<RequestAutoMergeCheckState, PostgresError> {
        request_auto_merge_check_state(self.db.as_ref(), intent).await
    }

    pub async fn claim_due_request_auto_merges(
        &self,
        command: ClaimDueRequestAutoMergesCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<Vec<ClaimedRequestAutoMerge>, PostgresError> {
        if command.limit == 0 {
            return Ok(Vec::new());
        }
        if command.lease_expires_at_unix <= command.now_unix {
            return Err(PostgresError::invalid_input(
                "auto-merge claim expiry must be in the future",
            ));
        }
        let now = u64_to_i64(command.now_unix, "auto-merge claim time")?;
        let lease_expires = u64_to_i64(command.lease_expires_at_unix, "auto-merge claim expiry")?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let query = entities::request_auto_merge_intent::Entity::find()
            .filter(entities::request_auto_merge_intent::Column::Status.eq("Active"))
            .filter(entities::request_auto_merge_intent::Column::NextAttemptAtUnix.lte(now))
            .filter(
                entities::request_auto_merge_intent::Column::ClaimExpiresAtUnix
                    .is_null()
                    .or(entities::request_auto_merge_intent::Column::ClaimExpiresAtUnix.lte(now)),
            );
        let rows = query
            .order_by_asc(entities::request_auto_merge_intent::Column::NextAttemptAtUnix)
            .order_by_asc(entities::request_auto_merge_intent::Column::CreatedAtUnix)
            .order_by_asc(entities::request_auto_merge_intent::Column::Id)
            .limit(command.limit)
            .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let repo = entities::repository::Entity::find_by_id(&row.repo_id)
                .one(&tx)
                .await
                .map_err(PostgresError::internal)?
                .ok_or_else(|| {
                    PostgresError::internal_message("auto-merge repository is missing")
                })?;
            let claim_token = generate_id(generated_ids, GeneratedIdKind::RequestAutoMergeClaim)?;
            let intent = row.try_into_domain()?;
            let attempt = row
                .attempt
                .checked_add(1)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| PostgresError::internal_message("auto-merge attempt overflow"))?;
            let mut update = row.into_active_model();
            update.claim_token = Set(Some(claim_token.clone()));
            update.claim_expires_at_unix = Set(Some(lease_expires));
            update.attempt = Set(i32::try_from(attempt).map_err(PostgresError::internal)?);
            update.update(&tx).await.map_err(PostgresError::internal)?;
            claimed.push(ClaimedRequestAutoMerge {
                intent,
                claim_token,
                claim_expires_at_unix: command.lease_expires_at_unix,
                attempt,
                owner: repo.owner_handle,
                name: repo.name,
            });
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(claimed)
    }

    pub async fn release_request_auto_merge_claim(
        &self,
        command: ReleaseRequestAutoMergeClaimCommand,
    ) -> Result<bool, PostgresError> {
        if command.next_attempt_at_unix < command.now_unix {
            return Err(PostgresError::invalid_input(
                "auto-merge next attempt cannot predate release",
            ));
        }
        let next = u64_to_i64(command.next_attempt_at_unix, "auto-merge next attempt time")?;
        let reset_attempt = command.last_error.is_none();
        let mut update = entities::request_auto_merge_intent::Entity::update_many()
            .col_expr(
                entities::request_auto_merge_intent::Column::ClaimToken,
                sea_orm::sea_query::Expr::value(Option::<String>::None),
            )
            .col_expr(
                entities::request_auto_merge_intent::Column::ClaimExpiresAtUnix,
                sea_orm::sea_query::Expr::value(Option::<i64>::None),
            )
            .col_expr(
                entities::request_auto_merge_intent::Column::NextAttemptAtUnix,
                sea_orm::sea_query::Expr::value(next),
            )
            .col_expr(
                entities::request_auto_merge_intent::Column::LastError,
                sea_orm::sea_query::Expr::value(command.last_error),
            );
        if reset_attempt {
            update = update.col_expr(
                entities::request_auto_merge_intent::Column::Attempt,
                sea_orm::sea_query::Expr::value(0),
            );
        }
        let result = update
            .filter(entities::request_auto_merge_intent::Column::Id.eq(command.intent_id))
            .filter(entities::request_auto_merge_intent::Column::Status.eq("Active"))
            .filter(entities::request_auto_merge_intent::Column::ClaimToken.eq(command.claim_token))
            .exec(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected == 1)
    }

    pub async fn stop_claimed_request_auto_merge(
        &self,
        command: StopClaimedRequestAutoMergeCommand,
    ) -> Result<Option<RequestAutoMergeMutation>, PostgresError> {
        let observed = entities::request_auto_merge_intent::Entity::find_by_id(&command.intent_id)
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        let Some(observed) = observed else {
            return Ok(None);
        };
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "request", &observed.request_id).await?;
        let row = entities::request_auto_merge_intent::Entity::find_by_id(&command.intent_id)
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let Some(row) = row.filter(|row| {
            row.status == "Active"
                && row.claim_token.as_deref() == Some(command.claim_token.as_str())
                && row
                    .claim_expires_at_unix
                    .is_some_and(|expires| expires >= command.now_unix as i64)
        }) else {
            return Ok(None);
        };
        let request = request_by_id(&tx, &row.request_id)
            .await?
            .ok_or_else(|| PostgresError::internal_message("auto-merge request is missing"))?;
        let intent = row.try_into_domain()?;
        let mutation = stop_request_auto_merge(
            &request,
            &intent,
            command.reason,
            command.event_id,
            command.now_unix,
        )?;
        persist_existing_auto_merge_mutation(&tx, row, &mutation).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(mutation))
    }
}

pub(super) struct StoredIntent {
    pub model: entities::request_auto_merge_intent::Model,
    pub intent: RequestAutoMergeIntent,
}

impl StoredIntent {
    pub(super) fn from_model(
        model: entities::request_auto_merge_intent::Model,
    ) -> Result<Self, PostgresError> {
        Ok(Self {
            intent: model.try_into_domain()?,
            model,
        })
    }
}

pub(super) async fn lock_active_intent_for_request(
    tx: &DatabaseTransaction,
    request_id: &str,
) -> Result<Option<StoredIntent>, PostgresError> {
    entities::request_auto_merge_intent::Entity::find()
        .filter(entities::request_auto_merge_intent::Column::RequestId.eq(request_id))
        .filter(entities::request_auto_merge_intent::Column::Status.eq("Active"))
        .lock_exclusive()
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
        .map(StoredIntent::from_model)
        .transpose()
}

pub(super) async fn lock_active_auto_merge_for_run(
    tx: &DatabaseTransaction,
    run_id: &str,
) -> Result<Option<StoredIntent>, PostgresError> {
    let Some(request_id) = request_id_for_check_run(tx, run_id).await? else {
        return Ok(None);
    };
    acquire_aggregate_lock(tx, "request", &request_id).await?;
    let Some(stored) = lock_active_intent_for_request(tx, &request_id).await? else {
        return Ok(None);
    };
    let state = request_auto_merge_check_state(tx, &stored.intent).await?;
    if state
        .evaluation
        .iter()
        .flat_map(|evaluation| &evaluation.checks)
        .any(|check| check.run_id.as_deref() == Some(run_id))
    {
        Ok(Some(stored))
    } else {
        Ok(None)
    }
}

pub(super) async fn request_id_for_check_run<C: ConnectionTrait>(
    conn: &C,
    run_id: &str,
) -> Result<Option<String>, PostgresError> {
    let row = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            SELECT evaluation.request_id
              FROM scope_request_check_evaluations evaluation
             WHERE evaluation.checks @>
                   jsonb_build_array(jsonb_build_object('run_id', $1::text))
             LIMIT 1
            "#,
            [run_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    row.map(|row| {
        row.try_get::<String>("", "request_id")
            .map_err(PostgresError::internal)
    })
    .transpose()
}

async fn lock_request_check_evidence(
    tx: &DatabaseTransaction,
    request_id: &str,
    head_oid: &str,
) -> Result<(), PostgresError> {
    let evaluation = entities::request_check_evaluation::Entity::find_by_id((
        request_id.to_string(),
        head_oid.to_string(),
    ))
    .one(tx)
    .await
    .map_err(PostgresError::internal)?
    .map(entities::request_check_evaluation::Model::try_into_domain)
    .transpose()?;
    let Some(evaluation) = evaluation else {
        return Ok(());
    };
    let mut run_ids = evaluation.run_ids().map(str::to_string).collect::<Vec<_>>();
    run_ids.sort_unstable();
    run_ids.dedup();
    for run_id in run_ids {
        let exists = entities::run::Entity::find_by_id(&run_id)
            .lock_exclusive()
            .one(tx)
            .await
            .map_err(PostgresError::internal)?
            .is_some();
        if !exists {
            return Err(PostgresError::conflict(
                "request check evidence is no longer available",
            ));
        }
    }
    Ok(())
}

pub(super) async fn stop_auto_merge_for_terminal_run(
    tx: &DatabaseTransaction,
    active: Option<StoredIntent>,
    run: &scope_domain::runs::run::Run,
) -> Result<(), PostgresError> {
    let Some(StoredIntent { model, intent }) = active else {
        return Ok(());
    };
    let request = request_by_id(tx, &intent.request_id)
        .await?
        .ok_or_else(|| PostgresError::internal_message("auto-merge request is missing"))?;
    if let Some(mutation) = stop_request_auto_merge_for_check_run(
        &request,
        &intent,
        run,
        automatic_event_id("stopped", &intent.id),
    )? {
        persist_existing_auto_merge_mutation(tx, model, &mutation).await?;
    }
    Ok(())
}

pub(super) async fn stop_auto_merges_for_revoked_actor(
    tx: &DatabaseTransaction,
    repo_id: &str,
    actor_user_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let mut rows = entities::request_auto_merge_intent::Entity::find()
        .filter(entities::request_auto_merge_intent::Column::RepoId.eq(repo_id))
        .filter(entities::request_auto_merge_intent::Column::ActorUserId.eq(actor_user_id))
        .filter(entities::request_auto_merge_intent::Column::Status.eq("Active"))
        .order_by_asc(entities::request_auto_merge_intent::Column::RequestId)
        .all(tx)
        .await
        .map_err(PostgresError::internal)?;
    for observed in rows.drain(..) {
        acquire_aggregate_lock(tx, "request", &observed.request_id).await?;
        let Some(stored) = entities::request_auto_merge_intent::Entity::find_by_id(&observed.id)
            .lock_exclusive()
            .one(tx)
            .await
            .map_err(PostgresError::internal)?
            .filter(|row| row.status == "Active")
        else {
            continue;
        };
        let intent = stored.try_into_domain()?;
        let request = request_by_id(tx, &intent.request_id)
            .await?
            .ok_or_else(|| PostgresError::internal_message("auto-merge request is missing"))?;
        let transition_time = now_unix
            .max(request.updated_at_unix)
            .max(intent.updated_at_unix);
        let mutation = stop_request_auto_merge(
            &request,
            &intent,
            RequestAutoMergeStopReason::AccessRevoked,
            automatic_event_id("stopped", &intent.id),
            transition_time,
        )?;
        persist_existing_auto_merge_mutation(tx, stored, &mutation).await?;
    }
    Ok(())
}

pub(super) async fn persist_new_auto_merge_mutation(
    tx: &DatabaseTransaction,
    mutation: &RequestAutoMergeMutation,
) -> Result<(), PostgresError> {
    save_request_row(tx, &mutation.request).await?;
    insert_request_event_row(tx, &mutation.event).await?;
    entities::request_auto_merge_intent::Model::from_domain(
        &mutation.intent,
        mutation.event.position,
    )?
    .into_active_model()
    .insert(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn persist_existing_auto_merge_mutation(
    tx: &DatabaseTransaction,
    stored: entities::request_auto_merge_intent::Model,
    mutation: &RequestAutoMergeMutation,
) -> Result<(), PostgresError> {
    save_request_row(tx, &mutation.request).await?;
    insert_request_event_row(tx, &mutation.event).await?;
    stored
        .with_transition(&mutation.intent)?
        .update(tx)
        .await
        .map_err(PostgresError::internal)?;
    Ok(())
}

pub(super) async fn request_auto_merge_check_state<C: sea_orm::ConnectionTrait>(
    conn: &C,
    intent: &RequestAutoMergeIntent,
) -> Result<RequestAutoMergeCheckState, PostgresError> {
    let evaluation = entities::request_check_evaluation::Entity::find_by_id((
        intent.request_id.clone(),
        intent.head_oid.clone(),
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    .map(entities::request_check_evaluation::Model::try_into_domain)
    .transpose()?;
    let run_ids = evaluation
        .iter()
        .flat_map(|evaluation| evaluation.checks.iter())
        .filter_map(|check| check.run_id.clone())
        .collect::<Vec<_>>();
    let run_states = if run_ids.is_empty() {
        Vec::new()
    } else {
        entities::run::Entity::find()
            .filter(entities::run::Column::Id.is_in(run_ids))
            .all(conn)
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| Ok((row.id.clone(), row.try_into_domain()?.state)))
            .collect::<Result<_, PostgresError>>()?
    };
    Ok(RequestAutoMergeCheckState {
        evaluation,
        run_states,
    })
}

pub(super) fn automatic_event_id(kind: &str, intent_id: &str) -> String {
    format!("request_auto_merge_{kind}_{intent_id}")
}

#[cfg(test)]
mod tests;
