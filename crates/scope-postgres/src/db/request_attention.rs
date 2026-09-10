use super::{
    RequestStore, entities,
    request_access::{ensure_user_exists, lock_request_repository},
    request_invitees::request_is_invitee,
};
use crate::error::PostgresError;
use scope_domain::requests::{
    ApplyRequestAttentionInput, Request, RequestAttention, RequestAttentionAction, RequestClaim,
    RequestQueueClassification, RequestQueueFacts, apply_request_attention_action,
    classify_request_queue_item, reactivate_request_attention,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, TransactionTrait,
    sea_query::OnConflict,
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct ApplyRequestAttentionCommand {
    pub repo_id: String,
    pub request_id: String,
    pub actor_user_id: String,
    pub expected_activity_version: u64,
    pub action: RequestAttentionAction,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct RequestAttentionResult {
    pub attention: RequestQueueClassification,
    pub claim: Option<RequestClaim>,
    pub activity_version: u64,
}

impl RequestStore {
    pub async fn apply_request_attention(
        &self,
        command: ApplyRequestAttentionCommand,
    ) -> Result<RequestAttentionResult, PostgresError> {
        let db = Arc::clone(&self.db);
        let tx = db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_request_repository(&tx, &command.request_id, &command.actor_user_id).await?;
        if request.repo_id != command.repo_id {
            return Err(PostgresError::not_found("request not found"));
        }
        ensure_user_exists(&tx, &command.actor_user_id).await?;
        let existing_attention =
            attention_for_user(&tx, &request.id, &command.actor_user_id).await?;
        let existing_claim = claim_for_request(&tx, &request.id).await?;
        let mutation = apply_request_attention_action(ApplyRequestAttentionInput {
            request: &request,
            actor_user_id: &command.actor_user_id,
            actor_is_maintainer: repo.access.is_maintainer(),
            expected_activity_version: command.expected_activity_version,
            existing_attention: existing_attention.as_ref(),
            existing_claim: existing_claim.as_ref(),
            action: command.action,
            now_unix: command.now_unix,
        })?;
        save_attention(&tx, &mutation.attention).await?;
        if matches!(command.action, RequestAttentionAction::Claim)
            && let Some(claim) = &mutation.claim
        {
            save_claim(&tx, claim).await?;
        }
        let is_invitee = request_is_invitee(&tx, &request.id, &command.actor_user_id).await?;
        let classification = classify_request_queue_item(RequestQueueFacts {
            request_state: request.state(),
            request_activity_version: request.activity_version,
            request_author_user_id: &request.author_user_id,
            viewer_user_id: Some(&command.actor_user_id),
            viewer_is_maintainer: repo.access.is_maintainer(),
            viewer_is_invitee: is_invitee,
            attention: Some(&mutation.attention),
            claim: mutation.claim.as_ref(),
            now_unix: command.now_unix,
        });
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RequestAttentionResult {
            attention: classification,
            claim: mutation.claim,
            activity_version: request.activity_version,
        })
    }
}

pub(super) async fn wait_after_own_reply<C: ConnectionTrait>(
    conn: &C,
    request: &Request,
    actor_user_id: &str,
    actor_is_maintainer: bool,
    reply_position: u64,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let existing_attention = attention_for_user(conn, &request.id, actor_user_id).await?;
    let existing_claim = claim_for_request(conn, &request.id).await?;
    let mutation = apply_request_attention_action(ApplyRequestAttentionInput {
        request,
        actor_user_id,
        actor_is_maintainer,
        expected_activity_version: reply_position,
        existing_attention: existing_attention.as_ref(),
        existing_claim: existing_claim.as_ref(),
        action: RequestAttentionAction::Wait,
        now_unix,
    })?;
    save_attention(conn, &mutation.attention).await
}

pub(super) async fn reactivate_attention_for_activity<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
    actor_user_id: &str,
    activity_version: u64,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let states = entities::request_attention_state::Entity::find()
        .filter(entities::request_attention_state::Column::RequestId.eq(request_id))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    for state in states {
        let state = state.try_into_domain()?;
        if let Some(active) =
            reactivate_request_attention(&state, actor_user_id, activity_version, now_unix)
        {
            save_attention(conn, &active).await?;
        }
    }
    Ok(())
}

async fn attention_for_user<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
    user_id: &str,
) -> Result<Option<RequestAttention>, PostgresError> {
    entities::request_attention_state::Entity::find_by_id((
        request_id.to_string(),
        user_id.to_string(),
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    .map(entities::request_attention_state::Model::try_into_domain)
    .transpose()
}

async fn claim_for_request<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
) -> Result<Option<RequestClaim>, PostgresError> {
    entities::request_claim::Entity::find_by_id(request_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::request_claim::Model::try_into_domain)
        .transpose()
}

async fn save_attention<C: ConnectionTrait>(
    conn: &C,
    attention: &RequestAttention,
) -> Result<(), PostgresError> {
    entities::request_attention_state::Entity::insert(
        entities::request_attention_state::Model::from_domain(attention)?.into_active_model(),
    )
    .on_conflict(
        OnConflict::columns([
            entities::request_attention_state::Column::RequestId,
            entities::request_attention_state::Column::UserId,
        ])
        .update_columns([
            entities::request_attention_state::Column::State,
            entities::request_attention_state::Column::Reason,
            entities::request_attention_state::Column::ThroughActivityVersion,
            entities::request_attention_state::Column::SnoozedUntilUnix,
            entities::request_attention_state::Column::UpdatedAtUnix,
        ])
        .to_owned(),
    )
    .exec(conn)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

async fn save_claim<C: ConnectionTrait>(
    conn: &C,
    claim: &RequestClaim,
) -> Result<(), PostgresError> {
    entities::request_claim::Entity::insert(
        entities::request_claim::Model::from_domain(claim)?.into_active_model(),
    )
    .on_conflict(
        OnConflict::column(entities::request_claim::Column::RequestId)
            .update_columns([
                entities::request_claim::Column::ClaimerUserId,
                entities::request_claim::Column::UpdatedAtUnix,
            ])
            .to_owned(),
    )
    .exec(conn)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}
