use super::{
    RepositoryCollaborationMutation, RepositoryStore, acquire_aggregate_lock,
    collaboration_rows::{lock_collaboration_state, save_collaboration_state},
    entities,
};
use crate::error::PostgresError;
use scope_domain::{
    repo_collaboration::issue_repository_invite_link,
    repo_invite_email::{
        INVITE_EMAIL_OWNER_WINDOW_SECS, InviteEmailAttempt, InviteEmailHistory,
        RepositoryInviteEmail, RepositoryInviteEmailState, RequestInviteEmailCommand,
        record_invite_email_attempt, request_repository_invite_email,
    },
    repository::{
        RepoRecord,
        collaboration::{CollaborationState, RepositoryInvite},
        repo_id,
    },
};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait, ExprTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
    sea_query::{LockBehavior, LockType},
};
use std::collections::BTreeMap;

use entities::repository_invite_email::{Column, Entity, Model, state_name};

pub struct RequestRepositoryInviteEmailCommand {
    pub owner: String,
    pub name: String,
    pub owner_user_id: String,
    pub invite_id: String,
    pub email_id: String,
    pub now_unix: u64,
}

pub struct RepositoryInviteEmailDelivery {
    pub record: RepoRecord,
    pub invite: RepositoryInvite,
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_repository_invite_email"))]
    pub async fn request_repository_invite_email(
        &self,
        command: RequestRepositoryInviteEmailCommand,
    ) -> Result<
        RepositoryCollaborationMutation<(RepositoryInvite, RepositoryInviteEmail)>,
        PostgresError,
    > {
        let repo_id = repo_id(&command.owner, &command.name);
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let mut repo = lock_collaboration_state(&tx, &repo_id)
            .await?
            .ok_or_else(|| PostgresError::not_found(format!("repo {repo_id} not found")))?;
        let before = repo.clone();
        let email = queue_invite_email(
            &tx,
            &repo,
            &command.owner_user_id,
            &command.invite_id,
            command.email_id,
            command.now_unix,
        )
        .await?;
        let invite = repo
            .collaboration
            .invitations
            .iter()
            .find(|invite| invite.id == email.invite_id)
            .cloned()
            .ok_or_else(|| PostgresError::internal_message("emailed invite is missing"))?;
        repo.record.bump_change_version();
        save_collaboration_state(&tx, &before, &repo).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RepositoryCollaborationMutation::committed(
            &repo.record,
            (invite, email),
        ))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_repository_invite_emails"))]
    pub async fn claim_due_repository_invite_emails(
        &self,
        claim_token: &str,
        now_unix: u64,
        lease_expires_at_unix: u64,
        limit: u64,
    ) -> Result<Vec<String>, PostgresError> {
        let now = to_i64(now_unix)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        Entity::delete_many()
            .filter(Column::InviteId.is_null())
            .filter(Column::CreatedAtUnix.lte(to_i64(
                now_unix.saturating_sub(INVITE_EMAIL_OWNER_WINDOW_SECS),
            )?))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let rows = Entity::find()
            .filter(Column::State.eq(state_name(RepositoryInviteEmailState::Queued)))
            .filter(Column::InviteId.is_not_null())
            .filter(Column::NextAttemptAtUnix.lte(now))
            .filter(
                Column::ClaimExpiresAtUnix
                    .is_null()
                    .or(Column::ClaimExpiresAtUnix.lte(now)),
            )
            .order_by_asc(Column::NextAttemptAtUnix)
            .order_by_asc(Column::CreatedAtUnix)
            .order_by_asc(Column::Id)
            .limit(limit)
            .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            claimed.push(row.id.clone());
            let mut active = row.into_active_model();
            active.claim_token = Set(Some(claim_token.to_string()));
            active.claim_expires_at_unix = Set(Some(to_i64(lease_expires_at_unix)?));
            active.update(&tx).await.map_err(PostgresError::internal)?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(claimed)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "issue_repository_invite_email_link"))]
    pub async fn issue_repository_invite_email_link(
        &self,
        email_id: &str,
        claim_token: &str,
        link_hash: String,
        now_unix: u64,
    ) -> Result<Option<RepositoryCollaborationMutation<RepositoryInviteEmailDelivery>>, PostgresError>
    {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(repo_id) = email_repository_id(&tx, email_id).await? else {
            return Ok(None);
        };
        let mut repo = lock_collaboration_state(&tx, &repo_id)
            .await?
            .ok_or_else(|| PostgresError::not_found(format!("repo {repo_id} not found")))?;
        let Some((_, email)) = claimed_email(&tx, email_id, claim_token).await? else {
            return Ok(None);
        };
        let before = repo.clone();
        let owner_user_id = repo.record.owner_user_id.clone();
        let invite = issue_repository_invite_link(
            &mut repo,
            &owner_user_id,
            &email.invite_id,
            link_hash,
            now_unix,
        )?;
        save_collaboration_state(&tx, &before, &repo).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(RepositoryCollaborationMutation::committed(
            &repo.record,
            RepositoryInviteEmailDelivery {
                record: repo.record.clone(),
                invite,
            },
        )))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "record_repository_invite_email_attempt"))]
    pub async fn record_repository_invite_email_attempt(
        &self,
        email_id: &str,
        claim_token: &str,
        attempt: InviteEmailAttempt,
        provider_message_id: Option<String>,
        now_unix: u64,
    ) -> Result<Option<RepositoryCollaborationMutation<RepositoryInviteEmail>>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(repo_id) = email_repository_id(&tx, email_id).await? else {
            return Ok(None);
        };
        let mut repo = lock_collaboration_state(&tx, &repo_id)
            .await?
            .ok_or_else(|| PostgresError::not_found(format!("repo {repo_id} not found")))?;
        let Some((row, mut email)) = claimed_email(&tx, email_id, claim_token).await? else {
            return Ok(None);
        };
        let retry_at = record_invite_email_attempt(&mut email, &attempt, now_unix);

        let mut active = row.into_active_model();
        active.state = Set(state_name(email.state).to_string());
        active.attempts = Set(i32::try_from(email.attempts).map_err(PostgresError::internal)?);
        active.updated_at_unix = Set(to_i64(email.updated_at_unix)?);
        active.claim_token = Set(None);
        active.claim_expires_at_unix = Set(None);
        if let Some(retry_at) = retry_at {
            active.next_attempt_at_unix = Set(to_i64(retry_at)?);
        }
        if provider_message_id.is_some() {
            active.provider_message_id = Set(provider_message_id);
        }
        active.last_error = Set(match attempt {
            InviteEmailAttempt::Accepted => None,
            InviteEmailAttempt::Retryable(error) | InviteEmailAttempt::Refused(error) => {
                Some(error.chars().take(2000).collect())
            }
        });
        active.update(&tx).await.map_err(PostgresError::internal)?;

        if retry_at.is_some() {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        }
        let before = repo.clone();
        repo.record.bump_change_version();
        save_collaboration_state(&tx, &before, &repo).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(RepositoryCollaborationMutation::committed(
            &repo.record,
            email,
        )))
    }
}

pub(super) async fn queue_invite_email(
    tx: &DatabaseTransaction,
    repo: &CollaborationState,
    owner_user_id: &str,
    invite_id: &str,
    email_id: String,
    now_unix: u64,
) -> Result<RepositoryInviteEmail, PostgresError> {
    acquire_aggregate_lock(tx, "repository-invite-email-owner", owner_user_id).await?;
    let for_invite = Entity::find()
        .filter(Column::InviteId.eq(invite_id.to_string()))
        .all(tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(Model::try_into_domain)
        .collect::<Result<Vec<_>, _>>()?;
    let window_start = to_i64(now_unix.saturating_sub(INVITE_EMAIL_OWNER_WINDOW_SECS))?;
    let owner_recent_sends_unix = Entity::find()
        .filter(Column::RequestedByUserId.eq(owner_user_id.to_string()))
        .filter(Column::CreatedAtUnix.gt(window_start))
        .filter(Column::State.ne(state_name(RepositoryInviteEmailState::Failed)))
        .all(tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| u64::try_from(row.created_at_unix).map_err(PostgresError::internal))
        .collect::<Result<Vec<_>, _>>()?;

    let email = request_repository_invite_email(
        repo,
        RequestInviteEmailCommand {
            id: email_id,
            owner_user_id,
            invite_id,
            now_unix,
        },
        InviteEmailHistory {
            for_invite: &for_invite,
            owner_recent_sends_unix: &owner_recent_sends_unix,
        },
    )?;
    Model {
        id: email.id.clone(),
        invite_id: Some(email.invite_id.clone()),
        requested_by_user_id: email.requested_by_user_id.clone(),
        state: state_name(email.state).to_string(),
        attempts: 0,
        next_attempt_at_unix: to_i64(now_unix)?,
        claim_token: None,
        claim_expires_at_unix: None,
        provider_message_id: None,
        last_error: None,
        created_at_unix: to_i64(email.created_at_unix)?,
        updated_at_unix: to_i64(email.updated_at_unix)?,
    }
    .into_active_model()
    .insert(tx)
    .await
    .map_err(PostgresError::internal)?;
    Ok(email)
}

async fn email_repository_id<C>(conn: &C, email_id: &str) -> Result<Option<String>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(invite_id) = Entity::find_by_id(email_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .and_then(|email| email.invite_id)
    else {
        return Ok(None);
    };
    Ok(entities::repository_invite::Entity::find_by_id(invite_id)
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(|invite| invite.repo_id))
}

async fn claimed_email(
    tx: &DatabaseTransaction,
    email_id: &str,
    claim_token: &str,
) -> Result<Option<(Model, RepositoryInviteEmail)>, PostgresError> {
    let Some(row) = Entity::find_by_id(email_id.to_string())
        .one(tx)
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    if row.state != state_name(RepositoryInviteEmailState::Queued)
        || row.claim_token.as_deref() != Some(claim_token)
    {
        return Ok(None);
    }
    let email = row.clone().try_into_domain()?;
    Ok(Some((row, email)))
}

fn to_i64(value: u64) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(PostgresError::internal)
}

pub(super) async fn latest_invite_emails<C>(
    conn: &C,
    invites: &[RepositoryInvite],
) -> Result<BTreeMap<String, RepositoryInviteEmail>, PostgresError>
where
    C: ConnectionTrait,
{
    if invites.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows = Entity::find()
        .filter(Column::InviteId.is_in(invites.iter().map(|invite| invite.id.clone())))
        .order_by_asc(Column::CreatedAtUnix)
        .order_by_asc(Column::Id)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let mut latest = BTreeMap::new();
    for row in rows {
        let email = row.try_into_domain()?;
        latest.insert(email.invite_id.clone(), email);
    }
    Ok(latest)
}
