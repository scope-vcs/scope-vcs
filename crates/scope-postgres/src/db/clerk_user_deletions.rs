//! Clerk users of deleted accounts, waiting to be deleted from Clerk. Rows are
//! written by account deletion and marked completed once Clerk confirms. A
//! failed attempt waits and tries again, so a Clerk outage only delays the
//! step. Completed rows keep refusing the Clerk user until tokens issued before
//! the deletion have expired, then they are purged.

use super::{AuthStore, entities};
use crate::error::PostgresError;
use entities::clerk_user_deletion::{ActiveModel, Column, Entity};
use scope_domain::account::deletion::{
    CLERK_USER_DELETION_TOMBSTONE_SECS, clerk_user_deletion_retry_at,
};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, EntityTrait, ExprTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait,
    sea_query::{Expr, LockBehavior, LockType},
};

impl AuthStore {
    /// Claims due deletions for one worker. A claim lapses by itself, so a
    /// deletion whose worker died is picked up again.
    pub async fn claim_due_clerk_user_deletions(
        &self,
        claim_token: &str,
        now_unix: u64,
        claim_expires_at_unix: u64,
        limit: u64,
    ) -> Result<Vec<String>, PostgresError> {
        let now = to_i64(now_unix)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let rows = Entity::find()
            .filter(Column::CompletedAtUnix.is_null())
            .filter(Column::NextAttemptAtUnix.lte(now))
            .filter(
                Column::ClaimExpiresAtUnix
                    .is_null()
                    .or(Column::ClaimExpiresAtUnix.lte(now)),
            )
            .order_by_asc(Column::NextAttemptAtUnix)
            .order_by_asc(Column::ClerkUserId)
            .limit(limit)
            .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            claimed.push(row.clerk_user_id.clone());
            let mut active = row.into_active_model();
            active.claim_token = Set(Some(claim_token.to_string()));
            active.claim_expires_at_unix = Set(Some(to_i64(claim_expires_at_unix)?));
            active.update(&tx).await.map_err(PostgresError::internal)?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(claimed)
    }

    /// Clerk no longer has the user. Ignored when the claim has lapsed and
    /// another worker holds the deletion.
    pub async fn complete_clerk_user_deletion(
        &self,
        clerk_user_id: &str,
        claim_token: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        Entity::update_many()
            .col_expr(Column::CompletedAtUnix, Expr::value(to_i64(now_unix)?))
            .col_expr(Column::ClaimToken, Expr::value(Option::<String>::None))
            .col_expr(Column::ClaimExpiresAtUnix, Expr::value(Option::<i64>::None))
            .filter(Column::ClerkUserId.eq(clerk_user_id))
            .filter(Column::ClaimToken.eq(claim_token))
            .exec(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    /// Forgets completed deletions once no token issued before them can
    /// still be valid.
    pub async fn purge_completed_clerk_user_deletions(
        &self,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let Some(cutoff) = now_unix.checked_sub(CLERK_USER_DELETION_TOMBSTONE_SECS) else {
            return Ok(());
        };
        Entity::delete_many()
            .filter(Column::CompletedAtUnix.lte(to_i64(cutoff)?))
            .exec(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    /// Records a failed attempt and schedules the next one.
    pub async fn retry_clerk_user_deletion(
        &self,
        clerk_user_id: &str,
        claim_token: &str,
        error: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let Some(row) = Entity::find_by_id(clerk_user_id.to_string())
            .filter(Column::ClaimToken.eq(claim_token))
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(());
        };
        let attempts = u32::try_from(row.attempts).map_err(PostgresError::internal)?;
        let mut active: ActiveModel = row.into_active_model();
        active.attempts = Set(i32::try_from(attempts.saturating_add(1)).unwrap_or(i32::MAX));
        active.next_attempt_at_unix =
            Set(to_i64(clerk_user_deletion_retry_at(attempts, now_unix))?);
        active.claim_token = Set(None);
        active.claim_expires_at_unix = Set(None);
        active.last_error = Set(Some(error.chars().take(2000).collect()));
        active
            .update(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }
}

/// Whether the Clerk user belongs to a deleted account. Until its Clerk
/// deletion finishes, a new account would be deleted along with the Clerk user;
/// afterwards, a token issued before the deletion must not recreate it.
pub(super) async fn clerk_user_deletion_recorded<C: ConnectionTrait>(
    conn: &C,
    clerk_user_id: &str,
) -> Result<bool, PostgresError> {
    Ok(Entity::find_by_id(clerk_user_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .is_some())
}

fn to_i64(value: u64) -> Result<i64, PostgresError> {
    i64::try_from(value).map_err(PostgresError::internal)
}
