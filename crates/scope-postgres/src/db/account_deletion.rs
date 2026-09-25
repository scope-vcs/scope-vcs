//! Deletes an account in one transaction. The domain decides what goes.
//! Owned repositories leave through repository deletion, which queues their
//! storage cleanup; other repositories lose the account the way they lose a
//! removed member; Clerk users are queued for deletion after the commit.
//! Authored work elsewhere survives through `ON DELETE SET NULL`.

use super::{
    AuthStore, GeneratedIdSource, acquire_aggregate_lock, auth::load_user_by_id, entities,
    repo_effects::save_repo_mutation, repo_lifecycle::delete_locked_repository,
    repository_from_model,
};
use crate::error::PostgresError;
use scope_domain::{
    account::deletion::{SharedRepositories, delete_account, forget_deleted_account},
    repo_actions::RepoEffects,
    repository::{RepositoryIncarnation, collaboration::normalize_repository_invite_email},
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect,
    TransactionTrait, sea_query::OnConflict,
};
use std::collections::BTreeSet;

#[derive(Debug)]
pub enum AccountDeletionError {
    /// The account owns repositories other members use.
    SharedRepositories(SharedRepositories),
    Persistence(PostgresError),
}

impl From<PostgresError> for AccountDeletionError {
    fn from(error: PostgresError) -> Self {
        Self::Persistence(error)
    }
}

/// A repository whose live views change with the deletion, and the version
/// that announces the change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountDeletionChange {
    pub incarnation: RepositoryIncarnation,
    pub change_version: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeletedAccount {
    pub deleted_repositories: Vec<AccountDeletionChange>,
    /// Repositories that lost the account's membership or invites.
    pub changed_repositories: Vec<AccountDeletionChange>,
}

impl AuthStore {
    pub async fn delete_account(
        &self,
        user_id: &str,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<DeletedAccount, AccountDeletionError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let user = load_user_by_id(&tx, user_id).await?;
        // A concurrent sign-in with this email waits, then finds the pending
        // Clerk deletion and is refused.
        acquire_aggregate_lock(&tx, "auth-email", &user.email).await?;

        let mut repository_ids = BTreeSet::new();
        repository_ids.extend(
            entities::repository::Entity::find()
                .select_only()
                .column(entities::repository::Column::Id)
                .filter(entities::repository::Column::OwnerUserId.eq(user_id))
                .into_tuple::<String>()
                .all(&tx)
                .await
                .map_err(PostgresError::internal)?,
        );
        repository_ids.extend(
            entities::repository_member::Entity::find()
                .select_only()
                .column(entities::repository_member::Column::RepoId)
                .filter(entities::repository_member::Column::UserId.eq(user_id))
                .into_tuple::<String>()
                .all(&tx)
                .await
                .map_err(PostgresError::internal)?,
        );
        repository_ids.extend(
            entities::repository_invite::Entity::find()
                .select_only()
                .column(entities::repository_invite::Column::RepoId)
                .filter(
                    Condition::any()
                        .add(
                            entities::repository_invite::Column::InvitedEmailNormalized
                                .eq(normalize_repository_invite_email(&user.email)),
                        )
                        .add(entities::repository_invite::Column::AcceptedByUserId.eq(user_id)),
                )
                .into_tuple::<String>()
                .all(&tx)
                .await
                .map_err(PostgresError::internal)?,
        );

        // Sorted, so two deletions sharing repositories cannot deadlock.
        let mut owned = Vec::new();
        let mut others = Vec::new();
        for repo_id in repository_ids {
            acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
            let Some(row) = entities::repository::Entity::find_by_id(repo_id)
                .one(&tx)
                .await
                .map_err(PostgresError::internal)?
            else {
                continue;
            };
            let repo = repository_from_model(&tx, row).await?;
            if repo.is_owner_user(user_id) {
                owned.push(repo);
            } else {
                others.push(repo);
            }
        }
        let identities = entities::auth_identity::Entity::find()
            .filter(entities::auth_identity::Column::UserId.eq(user_id))
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let deletion = delete_account(
            &user,
            &owned,
            identities
                .iter()
                .map(|identity| (identity.provider.as_str(), identity.subject.as_str())),
        )
        .map_err(AccountDeletionError::SharedRepositories)?;

        let mut deleted = DeletedAccount::default();
        for repo in &owned {
            let record = &repo.record;
            delete_locked_repository(
                &tx,
                repo,
                user_id,
                &record.owner_handle,
                &record.name,
                now_unix,
                generated_ids,
            )
            .await?;
            deleted.deleted_repositories.push(AccountDeletionChange {
                incarnation: repo.incarnation(),
                change_version: record.change_version.saturating_add(1),
            });
        }
        for mut repo in others {
            let before = repo.clone();
            let was_member = repo.member_for_user(user_id).is_some();
            if !forget_deleted_account(&mut repo, &user) {
                continue;
            }
            save_repo_mutation(
                &tx,
                &before,
                &repo,
                &RepoEffects::default(),
                now_unix,
                generated_ids,
            )
            .await?;
            if was_member {
                super::request_attention::remove_member_attention(&tx, &repo.record.id, user_id)
                    .await?;
                super::request_auto_merge::stop_auto_merges_for_revoked_actor(
                    &tx,
                    &repo.record.id,
                    user_id,
                    now_unix,
                )
                .await?;
            }
            deleted.changed_repositories.push(AccountDeletionChange {
                incarnation: repo.incarnation(),
                change_version: repo.record.change_version,
            });
        }

        let created_at_unix = i64::try_from(now_unix).map_err(PostgresError::internal)?;
        for clerk_user_id in deletion.clerk_user_ids {
            entities::clerk_user_deletion::Entity::insert(
                entities::clerk_user_deletion::ActiveModel {
                    clerk_user_id: Set(clerk_user_id),
                    attempts: Set(0),
                    next_attempt_at_unix: Set(created_at_unix),
                    claim_token: Set(None),
                    claim_expires_at_unix: Set(None),
                    last_error: Set(None),
                    created_at_unix: Set(created_at_unix),
                },
            )
            .on_conflict(
                OnConflict::column(entities::clerk_user_deletion::Column::ClerkUserId)
                    .do_nothing()
                    .to_owned(),
            )
            .try_insert()
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        }

        entities::user::Entity::delete_by_id(user_id.to_string())
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        // Sign-in lock rows are keyed by the email and sign-in identities.
        entities::metadata_lock::Entity::delete_many()
            .filter(entities::metadata_lock::Column::Key.is_in(
                std::iter::once(format!("auth-email:{}", user.email)).chain(identities.iter().map(
                    |identity| format!("auth-identity:{}:{}", identity.provider, identity.subject),
                )),
            ))
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests;
