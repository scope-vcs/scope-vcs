//! Deletes invites, and the email address each one names, once they have been
//! over for the domain's retention period.

use super::{
    GeneratedIdSource, RepositoryCollaborationMutation, RepositoryStore, acquire_aggregate_lock,
    entities, integer_columns, repo_effects::save_repo_mutation, repository_from_model,
};
use crate::error::PostgresError;
use scope_domain::repo_collaboration::{
    REPOSITORY_INVITE_RETENTION_SECS, prune_ended_repository_invites,
};
use sea_orm::{
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
    sea_query::Expr,
};

impl RepositoryStore {
    /// Repositories holding at least one invite that retention would remove.
    /// The domain makes the final decision under the repository lock.
    pub async fn repositories_with_prunable_invites(
        &self,
        now_unix: u64,
        limit: u64,
    ) -> Result<Vec<String>, PostgresError> {
        let Some(cutoff) = now_unix.checked_sub(REPOSITORY_INVITE_RETENTION_SECS) else {
            return Ok(Vec::new());
        };
        let cutoff = integer_columns::u64_to_i64(cutoff, "invite retention cutoff")?;
        entities::repository_invite::Entity::find()
            .select_only()
            .column(entities::repository_invite::Column::RepoId)
            .distinct()
            // Mirrors `RepositoryInvite::ended_at_unix`.
            .filter(Expr::cust_with_values(
                "COALESCE(revoked_at_unix, accepted_at_unix, expires_at_unix) <= $1",
                [cutoff],
            ))
            .order_by_asc(entities::repository_invite::Column::RepoId)
            .limit(limit)
            .into_tuple::<String>()
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)
    }

    /// Removes the repository's ended invites with their links and email
    /// delivery records. Returns how many invites went, or `None` when none did.
    pub async fn prune_ended_repository_invites(
        &self,
        repo_id: &str,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<Option<RepositoryCollaborationMutation<usize>>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let Some(row) = entities::repository::Entity::find_by_id(repo_id.to_string())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(None);
        };
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        let pruned = prune_ended_repository_invites(&mut repo, now_unix);
        if pruned.is_empty() {
            return Ok(None);
        }
        // Deleting an invite only clears its emails' invite_id, so they go
        // first. All of them predate the owner's daily email allowance window.
        entities::repository_invite_email::Entity::delete_many()
            .filter(
                entities::repository_invite_email::Column::InviteId
                    .is_in(pruned.iter().map(|invite| invite.id.clone())),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        // Deleting the invites cascades to their links.
        save_repo_mutation(
            &tx,
            &before,
            &repo,
            &Default::default(),
            now_unix,
            generated_ids,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(RepositoryCollaborationMutation::committed(
            &repo,
            pruned.len(),
        )))
    }
}

#[cfg(test)]
mod tests;
