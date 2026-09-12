use super::{
    GeneratedIdSource, RepositoryStore, acquire_aggregate_lock, auth::load_user_by_id, entities,
    repository_from_model, repository_rows::save_repository_delta,
};
use crate::error::PostgresError;
use scope_domain::{
    account::UserAccount,
    repo_collaboration::{
        AcceptRepositoryInviteOutcome, CreateRepositoryInviteCommand, accept_repository_invite,
        create_or_refresh_repository_invite, remove_repository_member, revoke_repository_invite,
        update_repository_member_permissions,
    },
    repository::collaboration::{
        RepositoryInvite, RepositoryMember, RepositoryMemberPermissions,
        normalize_repository_invite_email,
    },
    repository::{Repository, repo_id},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use std::collections::BTreeMap;

pub struct CreateRepositoryInviteMutation {
    pub owner: String,
    pub name: String,
    pub owner_user: UserAccount,
    pub invited_email: String,
    pub permissions: RepositoryMemberPermissions,
    pub invite_id: String,
    pub token_hash: String,
    pub now_unix: u64,
}

pub struct UpdateRepositoryMemberPermissionsCommand {
    pub owner: String,
    pub name: String,
    pub owner_user_id: String,
    pub member_user_id: String,
    pub permissions: RepositoryMemberPermissions,
    pub now_unix: u64,
}

impl RepositoryStore {
    pub async fn repository_collaboration(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<(Repository, BTreeMap<String, UserAccount>)>, PostgresError> {
        let Some(row) = entities::repository::Entity::find_by_id(repo_id(owner, name))
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(None);
        };
        let repo = repository_from_model(self.db.as_ref(), row).await?;
        let user_ids = repo
            .members
            .iter()
            .map(|member| member.user_id.clone())
            .collect::<Vec<_>>();
        let users = if user_ids.is_empty() {
            BTreeMap::new()
        } else {
            entities::user::Entity::find()
                .filter(entities::user::Column::Id.is_in(user_ids))
                .all(self.db.as_ref())
                .await
                .map_err(PostgresError::internal)?
                .into_iter()
                .map(|row| {
                    let user = row.try_into_domain()?;
                    Ok((user.id.clone(), user))
                })
                .collect::<Result<_, PostgresError>>()?
        };
        Ok(Some((repo, users)))
    }

    pub async fn user(&self, user_id: &str) -> Result<UserAccount, PostgresError> {
        load_user_by_id(self.db.as_ref(), user_id).await
    }

    pub async fn create_repository_invite(
        &self,
        command: CreateRepositoryInviteMutation,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepositoryInvite, PostgresError> {
        let repo_id = repo_id(&command.owner, &command.name);
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let row = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| {
                PostgresError::not_found(format!(
                    "repo {}/{} not found",
                    command.owner, command.name
                ))
            })?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        let invitee = user_by_normalized_email(&tx, &command.invited_email).await?;
        let invite = create_or_refresh_repository_invite(
            &mut repo,
            CreateRepositoryInviteCommand {
                id: command.invite_id,
                owner: &command.owner_user,
                invited_email: command.invited_email,
                invitee: invitee.as_ref(),
                permissions: command.permissions,
                token_hash: command.token_hash,
                now_unix: command.now_unix,
            },
        )?;
        save_repository_delta(&tx, &before, &repo, command.now_unix, generated_ids).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(invite)
    }

    pub async fn update_repository_member_permissions(
        &self,
        command: UpdateRepositoryMemberPermissionsCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepositoryMember, PostgresError> {
        let UpdateRepositoryMemberPermissionsCommand {
            owner,
            name,
            owner_user_id,
            member_user_id,
            permissions,
            now_unix,
        } = command;
        mutate_repository_collaboration(self, &owner, &name, now_unix, generated_ids, |repo| {
            update_repository_member_permissions(
                repo,
                &owner_user_id,
                &member_user_id,
                permissions,
                now_unix,
            )
            .map_err(PostgresError::from)
        })
        .await
    }

    pub async fn revoke_repository_invite(
        &self,
        owner: &str,
        name: &str,
        owner_user_id: &str,
        invite_id: &str,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepositoryInvite, PostgresError> {
        mutate_repository_collaboration(self, owner, name, now_unix, generated_ids, |repo| {
            revoke_repository_invite(repo, owner_user_id, invite_id, now_unix)
                .map_err(PostgresError::from)
        })
        .await
    }

    pub async fn remove_repository_member(
        &self,
        owner: &str,
        name: &str,
        owner_user_id: &str,
        member_user_id: &str,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepositoryMember, PostgresError> {
        let repo_id = repo_id(owner, name);
        let owner = owner.to_string();
        let name = name.to_string();
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let row = entities::repository::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        let removed = remove_repository_member(&mut repo, owner_user_id, member_user_id)
            .map_err(PostgresError::from)?;
        save_repository_delta(&tx, &before, &repo, now_unix, generated_ids).await?;
        super::request_attention::remove_member_attention(&tx, &repo_id, member_user_id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(removed)
    }

    pub async fn repository_invite_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<(scope_domain::repository::Repository, RepositoryInvite), PostgresError> {
        let invite = entities::repository_invite::Entity::find()
            .filter(entities::repository_invite::Column::TokenHash.eq(token_hash.to_string()))
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("repository invite not found"))?;
        let repo_row = entities::repository::Entity::find_by_id(invite.repo_id.clone())
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("repository invite repo is missing"))?;
        Ok((
            repository_from_model(self.db.as_ref(), repo_row).await?,
            invite.try_into_domain()?,
        ))
    }

    pub async fn accept_repository_invite(
        &self,
        token_hash: &str,
        user: UserAccount,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<(scope_domain::repository::Repository, RepositoryMember), PostgresError> {
        let token_hash = token_hash.to_string();
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository-invite-token", &token_hash).await?;
        let invite = entities::repository_invite::Entity::find()
            .filter(entities::repository_invite::Column::TokenHash.eq(token_hash.clone()))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("repository invite not found"))?;
        acquire_aggregate_lock(&tx, "repository", &invite.repo_id).await?;
        let row = entities::repository::Entity::find_by_id(invite.repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("repository invite not found"))?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        let outcome = accept_repository_invite(&mut repo, &user, &token_hash, now_unix)?;
        save_repository_delta(&tx, &before, &repo, now_unix, generated_ids).await?;
        let result = match outcome {
            AcceptRepositoryInviteOutcome::Accepted(member) => Ok((repo, member)),
            AcceptRepositoryInviteOutcome::Expired => {
                Err(PostgresError::conflict("repository invite expired"))
            }
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        result
    }
}

async fn mutate_repository_collaboration<T, F>(
    store: &RepositoryStore,
    owner: &str,
    name: &str,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
    op: F,
) -> Result<T, PostgresError>
where
    F: FnOnce(&mut Repository) -> Result<T, PostgresError>,
{
    let repo_id = repo_id(owner, name);
    let tx = store.db.begin().await.map_err(PostgresError::internal)?;
    acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
    let row = entities::repository::Entity::find_by_id(repo_id)
        .one(&tx)
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))?;
    let mut repo = repository_from_model(&tx, row).await?;
    let before = repo.clone();
    let result = op(&mut repo)?;
    save_repository_delta(&tx, &before, &repo, now_unix, generated_ids).await?;
    tx.commit().await.map_err(PostgresError::internal)?;
    Ok(result)
}

async fn user_by_normalized_email<C>(
    conn: &C,
    email: &str,
) -> Result<Option<UserAccount>, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let normalized = normalize_repository_invite_email(email);
    entities::user::Entity::find()
        .filter(entities::user::Column::Email.eq(normalized))
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(entities::user::Model::try_into_domain)
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{
        CatalogFixture, MetadataStore, TestDatabaseTarget, generated_ids::test_generated_id,
        locks::wait_for_transaction_waiter,
    };
    use scope_domain::{
        policy::Visibility,
        repository::{RepoLifecycleState, collaboration::RepositoryInviteState},
    };
    use sea_orm::{
        ActiveModelTrait, ConnectionTrait, DatabaseBackend, IntoActiveModel, Statement,
        TransactionTrait,
    };
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invite_creation_resolves_invitee_after_waiting_for_repository_lock() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        let owner = UserAccount {
            id: "invite_owner".into(),
            handle: "owner".into(),
            email: "owner@example.com".into(),
            email_verified: true,
        };
        let member = UserAccount {
            id: "invite_member".into(),
            handle: "member".into(),
            email: "member@example.com".into(),
            email_verified: true,
        };
        let mut repo = Repository::new(
            &owner,
            "repo",
            Visibility::Private,
            "repoi_invite_lock_test",
        )
        .unwrap();
        repo.record.lifecycle_state = RepoLifecycleState::Ready;
        let existing_invite = create_or_refresh_repository_invite(
            &mut repo,
            CreateRepositoryInviteCommand {
                id: "invite_existing".into(),
                owner: &owner,
                invited_email: member.email.clone(),
                invitee: None,
                permissions: RepositoryMemberPermissions::default(),
                token_hash: "token_existing".into(),
                now_unix: 1_700_000_000,
            },
        )
        .unwrap();
        let mut catalog = CatalogFixture::default();
        catalog.users.insert(owner.id.clone(), owner.clone());
        catalog.repositories.insert(repo.record.id.clone(), repo);
        store.admin().seed_catalog_for_tests(catalog).unwrap();

        let held = store.db.begin().await.unwrap();
        acquire_aggregate_lock(&held, "repository", "owner/repo")
            .await
            .unwrap();
        let holder_pid = held
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT pg_backend_pid() AS pid".to_string(),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i32>("", "pid")
            .unwrap();
        let row = entities::repository::Entity::find_by_id("owner/repo")
            .one(&held)
            .await
            .unwrap()
            .unwrap();
        let mut locked_repo = repository_from_model(&held, row).await.unwrap();
        let before = locked_repo.clone();

        let create_store = store.clone();
        let create_owner = owner.clone();
        let create = tokio::spawn(async move {
            create_store
                .repositories()
                .create_repository_invite(
                    CreateRepositoryInviteMutation {
                        owner: "owner".into(),
                        name: "repo".into(),
                        owner_user: create_owner,
                        invited_email: "member@example.com".into(),
                        permissions: RepositoryMemberPermissions::default(),
                        invite_id: "invite_invalid".into(),
                        token_hash: "token_invalid".into(),
                        now_unix: 1_700_000_001,
                    },
                    &test_generated_id,
                )
                .await
        });
        wait_for_transaction_waiter(&store, holder_pid).await;
        assert!(!create.is_finished());

        entities::user::Model::from_domain(&member)
            .into_active_model()
            .insert(&held)
            .await
            .unwrap();
        let outcome = accept_repository_invite(
            &mut locked_repo,
            &member,
            &existing_invite.token_hash,
            1_700_000_001,
        )
        .unwrap();
        assert!(matches!(
            outcome,
            AcceptRepositoryInviteOutcome::Accepted(_)
        ));
        save_repository_delta(
            &held,
            &before,
            &locked_repo,
            1_700_000_001,
            &test_generated_id,
        )
        .await
        .unwrap();
        held.commit().await.unwrap();

        let error = tokio::time::timeout(Duration::from_secs(60), create)
            .await
            .expect("invite creation should resume after the repository lock is released")
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
        assert_eq!(error.message, "user is already a repository member");
        let persisted = store
            .repositories()
            .repository("owner", "repo")
            .await
            .unwrap()
            .unwrap();
        assert!(persisted.member_for_user(&member.id).is_some());
        assert!(persisted.invitations.iter().all(|invite| {
            invite.id != "invite_invalid"
                && !(invite.state == RepositoryInviteState::Pending
                    && invite.invited_email_normalized == "member@example.com")
        }));
    }
}
