//! Deletes an account in one transaction. The domain decides what goes.
//! Owned repositories leave through repository deletion, which queues their
//! storage cleanup; other repositories lose the account the way they lose a
//! removed member; Clerk users are queued for deletion after the commit.
//! Authored work elsewhere survives through `ON DELETE SET NULL`.

use super::{
    AuthStore, GeneratedIdSource, acquire_aggregate_lock,
    auth::load_user_by_id,
    entities,
    repo_effects::save_repo_mutation,
    repo_lifecycle::delete_locked_repository,
    repository_from_model,
    request_revision_rows::revisions_for_request_ids,
    request_rows::{request_by_id, request_events_by_request_id},
    requests::persist_deleted_draft,
};
use crate::error::PostgresError;
use scope_domain::{
    account::deletion::{SharedRepositories, delete_account, forget_deleted_account},
    repo_actions::RepoEffects,
    repository::{
        Repository, RepositoryIncarnation, collaboration::normalize_repository_invite_email,
    },
    requests::{CloseRequestInput, CloseRequestMutation, close_request},
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DatabaseBackend,
    DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Statement,
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
    /// Other repositories whose requests, discussions or runs now show a
    /// deleted user, or lost the account's drafts.
    pub contributed_repositories: Vec<RepositoryIncarnation>,
}

/// Every repository holding work the account did, besides membership.
const CONTRIBUTED_REPOSITORIES_SQL: &str = r#"
    SELECT repo_id FROM scope_requests
        WHERE $1 IN (author_user_id, closed_by_user_id, merged_by_user_id)
    UNION SELECT r.repo_id FROM scope_request_events e
        JOIN scope_requests r ON r.id = e.request_id WHERE e.actor_user_id = $1
    UNION SELECT r.repo_id FROM scope_request_revisions v
        JOIN scope_requests r ON r.id = v.request_id WHERE v.actor_user_id = $1
    UNION SELECT r.repo_id FROM scope_request_discussions d
        JOIN scope_requests r ON r.id = d.request_id
        WHERE $1 IN (d.author_user_id, d.resolved_by_user_id)
    UNION SELECT r.repo_id FROM scope_request_discussion_replies x
        JOIN scope_request_discussions d ON d.id = x.discussion_id
        JOIN scope_requests r ON r.id = d.request_id WHERE x.author_user_id = $1
    UNION SELECT r.repo_id FROM scope_request_ratings g
        JOIN scope_requests r ON r.id = g.request_id
        WHERE $1 IN (g.rater_user_id, g.subject_user_id)
    UNION SELECT r.repo_id FROM scope_request_invitees i
        JOIN scope_requests r ON r.id = i.request_id
        WHERE $1 IN (i.user_id, i.invited_by_user_id)
    UNION SELECT repo_id FROM scope_runs WHERE requested_by_user_id = $1
"#;

impl AuthStore {
    pub async fn delete_account(
        &self,
        user_id: &str,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<DeletedAccount, AccountDeletionError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let user = load_user_by_id(&tx, user_id).await?;
        let identities = entities::auth_identity::Entity::find()
            .filter(entities::auth_identity::Column::UserId.eq(user_id))
            .order_by_asc(entities::auth_identity::Column::Provider)
            .order_by_asc(entities::auth_identity::Column::Subject)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        // Sign-in takes the identity lock, then the email lock; taking them in
        // the same order cannot deadlock. A concurrent sign-in waits, then
        // finds the recorded Clerk deletion and is refused.
        for identity in &identities {
            let key = format!("{}:{}", identity.provider, identity.subject);
            acquire_aggregate_lock(&tx, "auth-identity", &key).await?;
        }
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

        // The account's drafts elsewhere could never be deleted once their
        // author is gone, so they go with the account.
        let drafts = entities::request::Entity::find()
            .select_only()
            .column(entities::request::Column::Id)
            .column(entities::request::Column::RepoId)
            .filter(entities::request::Column::AuthorUserId.eq(user_id))
            .filter(entities::request::Column::SubmittedAtUnix.is_null())
            .into_tuple::<(String, String)>()
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        repository_ids.extend(drafts.iter().map(|(_, repo_id)| repo_id.clone()));

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
        let contributed_repository_ids: BTreeSet<String> = tx
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                CONTRIBUTED_REPOSITORIES_SQL,
                [user_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| row.try_get::<String>("", "repo_id"))
            .collect::<Result<_, _>>()
            .map_err(PostgresError::internal)?;
        for (request_id, repo_id) in drafts {
            let Some(repo) = others.iter().find(|repo| repo.record.id == repo_id) else {
                // Drafts in owned repositories leave with the repository.
                continue;
            };
            delete_draft(&tx, repo, &request_id, user_id, now_unix, generated_ids).await?;
        }
        for mut repo in others {
            let before = repo.clone();
            let was_member = repo.member_for_user(user_id).is_some();
            if !forget_deleted_account(&mut repo, &user) {
                if contributed_repository_ids.contains(&repo.record.id) {
                    deleted.contributed_repositories.push(repo.incarnation());
                }
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

        let notified: BTreeSet<&str> = deleted
            .deleted_repositories
            .iter()
            .chain(&deleted.changed_repositories)
            .map(|change| change.incarnation.repository_id())
            .chain(
                deleted
                    .contributed_repositories
                    .iter()
                    .map(RepositoryIncarnation::repository_id),
            )
            .collect();
        let unlocked: Vec<String> = contributed_repository_ids
            .iter()
            .filter(|repo_id| !notified.contains(repo_id.as_str()))
            .cloned()
            .collect();
        drop(notified);
        deleted.contributed_repositories.extend(
            entities::repository::Entity::find()
                .filter(entities::repository::Column::Id.is_in(unlocked))
                .all(&tx)
                .await
                .map_err(PostgresError::internal)?
                .into_iter()
                .map(|row| RepositoryIncarnation::new(row.id, row.incarnation_id))
                .collect::<Result<Vec<_>, _>>()
                .map_err(PostgresError::internal)?,
        );

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
                    completed_at_unix: Set(None),
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

/// Deletes one of the account's drafts in a repository it does not own, the
/// way its author closing it would.
async fn delete_draft(
    tx: &DatabaseTransaction,
    repo: &Repository,
    request_id: &str,
    user_id: &str,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError> {
    acquire_aggregate_lock(tx, "request", request_id).await?;
    let Some(request) = request_by_id(tx, request_id).await? else {
        return Ok(());
    };
    let events = request_events_by_request_id(tx, request_id).await?;
    let revisions = revisions_for_request_ids(tx, std::slice::from_ref(&request.id)).await?;
    let mutation = close_request(
        request,
        events,
        revisions,
        CloseRequestInput {
            request_id: request_id.to_string(),
            actor_user_id: user_id.to_string(),
            actor_is_maintainer: false,
            // Deleting a draft records no event.
            event_id: format!("event_request_closed_{request_id}"),
            now_unix,
        },
    )?;
    let CloseRequestMutation::DeletedDraft {
        request,
        revisions,
        orphan_objects,
        ..
    } = mutation
    else {
        return Err(PostgresError::internal_message(
            "an unsubmitted request was closed instead of deleted",
        ));
    };
    persist_deleted_draft(
        tx,
        &repo.incarnation(),
        &request,
        &revisions,
        orphan_objects,
        now_unix,
        generated_ids,
    )
    .await
}

#[cfg(test)]
mod tests;
