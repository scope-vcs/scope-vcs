use super::{
    RepositoryCollaborationMutation, RepositoryStore,
    collaboration_rows::{lock_collaboration_state, save_collaboration_state},
    entities, integer_columns,
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

    pub async fn prune_ended_repository_invites(
        &self,
        repo_id: &str,
        now_unix: u64,
    ) -> Result<Option<RepositoryCollaborationMutation<usize>>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(mut repo) = lock_collaboration_state(&tx, repo_id).await? else {
            return Ok(None);
        };
        let before = repo.clone();
        let pruned = prune_ended_repository_invites(&mut repo, now_unix);
        if pruned.is_empty() {
            return Ok(None);
        }
        entities::repository_invite_email::Entity::delete_many()
            .filter(
                entities::repository_invite_email::Column::InviteId
                    .is_in(pruned.iter().map(|invite| invite.id.clone())),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        save_collaboration_state(&tx, &before, &repo).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(RepositoryCollaborationMutation::committed(
            &repo.record,
            pruned.len(),
        )))
    }
}

#[cfg(test)]
mod tests;
