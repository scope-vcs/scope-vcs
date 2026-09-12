use super::{
    AuthStore, acquire_aggregate_lock,
    cli_auth_results::{DeviceLoginPoll, NewCliSession, StartDeviceLoginCommand},
    cli_sessions::insert_cli_session_in_tx,
    entities,
    integer_columns::{i64_to_u64, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::{account::UserAccount, account::cli_auth as cli_auth_rules};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter,
    TransactionTrait, sea_query::Expr,
};
use std::collections::{BTreeMap, BTreeSet};

impl AuthStore {
    pub async fn users_by_ids(
        &self,
        user_ids: impl IntoIterator<Item = String>,
    ) -> Result<BTreeMap<String, UserAccount>, PostgresError> {
        load_users_by_ids(self.db.as_ref(), user_ids).await
    }

    pub async fn start_cli_device_login(
        &self,
        command: StartDeviceLoginCommand,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-auth", "start").await?;
        cleanup_expired_cli_rows(&tx, now_unix).await?;
        enforce_device_login_start_limits(&tx, now_unix).await?;
        entities::cli_device_login::Model {
            device_code_hash: command.device_code_hash,
            user_code_hash: command.user_code_hash,
            created_at_unix: u64_to_i64(command.created_at_unix, "CLI login creation time")?,
            expires_at_unix: u64_to_i64(command.expires_at_unix, "CLI login expiry")?,
            completed_user_id: None,
            completed_at_unix: None,
            consumed_at_unix: None,
        }
        .into_active_model()
        .insert(&tx)
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn complete_cli_device_login_by_user_code_hash(
        &self,
        user_code_hash: &str,
        user: &UserAccount,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let user_code_hash = user_code_hash.to_string();
        let user_id = user.id.clone();
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-device-user-code", &user_code_hash).await?;

        let Some(login) = entities::cli_device_login::Entity::find()
            .filter(entities::cli_device_login::Column::UserCodeHash.eq(user_code_hash))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::not_found("CLI login code not found"));
        };

        match cli_auth_rules::decide_device_login_completion(
            cli_auth_rules::DeviceLoginCompletionState {
                expires_at_unix: i64_to_u64(login.expires_at_unix, "CLI login expiry")?,
                completed: login.completed_user_id.is_some(),
            },
            now_unix,
        )? {
            cli_auth_rules::DeviceLoginCompletionDecision::Expired => {
                entities::cli_device_login::Entity::delete_by_id(login.device_code_hash)
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                return Err(PostgresError::conflict("CLI login code expired"));
            }
            cli_auth_rules::DeviceLoginCompletionDecision::Complete => {}
        }

        cleanup_expired_cli_rows(&tx, now_unix).await?;
        entities::cli_device_login::Entity::update_many()
            .filter(entities::cli_device_login::Column::DeviceCodeHash.eq(login.device_code_hash))
            .col_expr(
                entities::cli_device_login::Column::CompletedUserId,
                Expr::value(user_id),
            )
            .col_expr(
                entities::cli_device_login::Column::CompletedAtUnix,
                Expr::value(u64_to_i64(now_unix, "current time")?),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn poll_cli_device_login_by_hash(
        &self,
        device_code_hash: &str,
        session: NewCliSession,
        now_unix: u64,
    ) -> Result<DeviceLoginPoll, PostgresError> {
        let device_code_hash = device_code_hash.to_string();
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-device-code", &device_code_hash).await?;

        let Some(login) = entities::cli_device_login::Entity::find_by_id(device_code_hash)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::not_found("CLI device login not found"));
        };
        match cli_auth_rules::decide_device_login_poll(
            cli_auth_rules::DeviceLoginPollState {
                expires_at_unix: i64_to_u64(login.expires_at_unix, "CLI login expiry")?,
                consumed: login.consumed_at_unix.is_some(),
                completed_user_id: login.completed_user_id.clone(),
            },
            now_unix,
        )? {
            cli_auth_rules::DeviceLoginPollDecision::Expired => {
                entities::cli_device_login::Entity::delete_by_id(login.device_code_hash)
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                Err(PostgresError::conflict("CLI device login expired"))
            }
            cli_auth_rules::DeviceLoginPollDecision::Pending { expires_at_unix } => {
                tx.commit().await.map_err(PostgresError::internal)?;
                Ok(DeviceLoginPoll::Pending { expires_at_unix })
            }
            cli_auth_rules::DeviceLoginPollDecision::Complete { user_id } => {
                cleanup_expired_cli_rows(&tx, now_unix).await?;
                let identity = insert_cli_session_in_tx(&tx, &user_id, session).await?;
                entities::cli_device_login::Entity::update_many()
                    .filter(
                        entities::cli_device_login::Column::DeviceCodeHash
                            .eq(login.device_code_hash),
                    )
                    .col_expr(
                        entities::cli_device_login::Column::ConsumedAtUnix,
                        Expr::value(u64_to_i64(now_unix, "current time")?),
                    )
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                tx.commit().await.map_err(PostgresError::internal)?;
                Ok(DeviceLoginPoll::Complete { identity })
            }
        }
    }

    pub async fn verify_cli_session_by_hash(
        &self,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<UserAccount, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(session) = entities::cli_session::Entity::find()
            .filter(entities::cli_session::Column::TokenHash.eq(token_hash))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::unauthenticated("invalid CLI token"));
        };
        let user_id = match cli_auth_rules::decide_cli_session_use(
            cli_auth_rules::CliSessionState {
                expires_at_unix: i64_to_u64(session.expires_at_unix, "CLI session expiry")?,
                revoked: session.revoked_at_unix.is_some(),
                user_id: session.user_id.clone(),
            },
            now_unix,
        )? {
            cli_auth_rules::CliSessionUseDecision::Expired => {
                return Err(PostgresError::unauthenticated("CLI token expired"));
            }
            cli_auth_rules::CliSessionUseDecision::Active { user_id } => user_id,
        };
        let user = load_user_by_id(&tx, &user_id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(user)
    }

    pub async fn revoke_cli_session_by_hash(
        &self,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let token_hash = token_hash.to_string();
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "cli-session-token", &token_hash).await?;
        let Some(session) = entities::cli_session::Entity::find()
            .filter(entities::cli_session::Column::TokenHash.eq(token_hash))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Err(PostgresError::unauthenticated("invalid CLI token"));
        };
        match cli_auth_rules::decide_cli_session_revoke(
            i64_to_u64(session.expires_at_unix, "CLI session expiry")?,
            now_unix,
        ) {
            cli_auth_rules::CliSessionRevokeDecision::Expired => {
                entities::cli_session::Entity::delete_by_id(session.id)
                    .exec(&tx)
                    .await
                    .map_err(PostgresError::internal)?;
                return Err(PostgresError::unauthenticated("CLI token expired"));
            }
            cli_auth_rules::CliSessionRevokeDecision::Revoke => {}
        }
        entities::cli_session::Entity::update_many()
            .filter(entities::cli_session::Column::Id.eq(session.id))
            .col_expr(
                entities::cli_session::Column::RevokedAtUnix,
                Expr::value(u64_to_i64(now_unix, "current time")?),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(())
    }
}

pub async fn cleanup_expired_cli_rows<C>(conn: &C, now_unix: u64) -> Result<(), PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let now_unix = u64_to_i64(now_unix, "current time")?;
    entities::cli_device_login::Entity::delete_many()
        .filter(entities::cli_device_login::Column::ExpiresAtUnix.lte(now_unix))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    entities::cli_browser_login::Entity::delete_many()
        .filter(entities::cli_browser_login::Column::ExpiresAtUnix.lte(now_unix))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    entities::cli_exchange_grant::Entity::delete_many()
        .filter(entities::cli_exchange_grant::Column::ExpiresAtUnix.lte(now_unix))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    entities::cli_session::Entity::delete_many()
        .filter(entities::cli_session::Column::ExpiresAtUnix.lte(now_unix))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok(())
}

async fn enforce_device_login_start_limits<C>(conn: &C, now_unix: u64) -> Result<(), PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let (pending_count, window_count) =
        login_start_counts::<entities::cli_device_login::Entity, _>(
            conn,
            entities::cli_device_login::Column::CreatedAtUnix,
            cli_auth_rules::device_login_start_window_start(now_unix),
        )
        .await?;
    Ok(cli_auth_rules::enforce_device_login_start_rate_limit(
        pending_count,
        window_count,
    )?)
}

/// Counts every pending login row of one kind and the subset created inside
/// the rate-limit window. The domain keeps a separate limit per login kind,
/// so callers pass the result to that kind's rule.
pub(super) async fn login_start_counts<E, C>(
    conn: &C,
    created_at: E::Column,
    window_start_unix: u64,
) -> Result<(u64, u64), PostgresError>
where
    E: EntityTrait,
    E::Model: sea_orm::FromQueryResult + Send + Sync,
    C: sea_orm::ConnectionTrait,
{
    let pending_count = E::find()
        .count(conn)
        .await
        .map_err(PostgresError::internal)?;
    let window_start = u64_to_i64(window_start_unix, "login rate-limit window start")?;
    let window_count = E::find()
        .filter(created_at.gte(window_start))
        .count(conn)
        .await
        .map_err(PostgresError::internal)?;
    Ok((pending_count, window_count))
}

pub async fn load_users_by_ids<C>(
    conn: &C,
    user_ids: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, UserAccount>, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let user_ids = user_ids.into_iter().collect::<BTreeSet<_>>();
    if user_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    entities::user::Entity::find()
        .filter(entities::user::Column::Id.is_in(user_ids))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| {
            let user = row.try_into_domain()?;
            Ok((user.id.clone(), user))
        })
        .collect()
}

pub async fn load_user_by_id<C>(conn: &C, user_id: &str) -> Result<UserAccount, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    entities::user::Entity::find_by_id(user_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("signed-in user was not persisted"))?
        .try_into_domain()
}
