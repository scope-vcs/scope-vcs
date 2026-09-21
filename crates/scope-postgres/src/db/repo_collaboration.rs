use super::{
    GeneratedIdSource, RepositoryStore, acquire_aggregate_lock, auth::load_user_by_id, entities,
    repo_effects::save_repo_mutation, repository_from_model,
};
use crate::error::{PostgresError, PostgresErrorKind};
use scope_domain::{
    account::UserAccount,
    repo_collaboration::{
        AcceptRepositoryInviteOutcome, CreateRepositoryInviteCommand, accept_repository_invite,
        create_repository_invite, issue_repository_invite_link, remove_repository_member,
        revoke_repository_invite, update_repository_member_permissions,
    },
    repo_invite_email::RepositoryInviteEmail,
    repository::collaboration::{
        RepositoryInvite, RepositoryMember, RepositoryMemberPermissions,
        normalize_repository_invite_email,
    },
    repository::{Repository, RepositoryIncarnation, repo_id},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use std::collections::BTreeMap;

pub struct RepositoryCollaborationMutation<T> {
    pub incarnation: RepositoryIncarnation,
    pub change_version: u64,
    pub value: T,
}

impl<T> RepositoryCollaborationMutation<T> {
    pub(super) fn committed(repo: &Repository, value: T) -> Self {
        Self {
            incarnation: repo.incarnation(),
            change_version: repo.record.change_version,
            value,
        }
    }
}

pub struct CreateRepositoryInviteMutation {
    pub owner: String,
    pub name: String,
    pub owner_user: UserAccount,
    pub invited_email: String,
    pub permissions: RepositoryMemberPermissions,
    pub invite_id: String,
    pub email_id: String,
    pub now_unix: u64,
}

pub struct IssueRepositoryInviteLinkCommand {
    pub owner: String,
    pub name: String,
    pub owner_user_id: String,
    pub invite_id: String,
    pub link_hash: String,
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
    ) -> Result<
        RepositoryCollaborationMutation<(RepositoryInvite, Option<RepositoryInviteEmail>)>,
        PostgresError,
    > {
        let now_unix = command.now_unix;
        let repo_id = repo_id(&command.owner, &command.name);
        let owner_name = command.owner.clone();
        let name = command.name.clone();
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let row = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| {
                PostgresError::not_found(format!("repo {owner_name}/{name} not found"))
            })?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        let invitee = user_by_normalized_email(&tx, &command.invited_email).await?;
        let mutation = create_repository_invite(
            &mut repo,
            CreateRepositoryInviteCommand {
                id: command.invite_id,
                owner: &command.owner_user,
                invited_email: command.invited_email,
                invitee: invitee.as_ref(),
                permissions: command.permissions,
                now_unix: command.now_unix,
            },
        )?;
        save_repo_mutation(
            &tx,
            &before,
            &repo,
            &mutation_effects_none(),
            now_unix,
            generated_ids,
        )
        .await?;
        // The invite and its first email commit together, so an invite is
        // never saved with a forgotten email. An owner who has used up the
        // daily email allowance still gets the invite, and can copy a link.
        let email = match super::repo_invite_emails::queue_invite_email(
            &tx,
            &repo,
            &command.owner_user.id,
            &mutation.id,
            command.email_id,
            now_unix,
        )
        .await
        {
            Ok(email) => Some(email),
            Err(error) if error.kind == PostgresErrorKind::ResourceExhausted => None,
            Err(error) => return Err(error),
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RepositoryCollaborationMutation::committed(
            &repo,
            (mutation, email),
        ))
    }

    pub async fn issue_repository_invite_link(
        &self,
        command: IssueRepositoryInviteLinkCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepositoryCollaborationMutation<RepositoryInvite>, PostgresError> {
        let IssueRepositoryInviteLinkCommand {
            owner,
            name,
            owner_user_id,
            invite_id,
            link_hash,
            now_unix,
        } = command;
        mutate_repository_collaboration(self, &owner, &name, now_unix, generated_ids, move |repo| {
            issue_repository_invite_link(repo, &owner_user_id, &invite_id, link_hash, now_unix)
                .map_err(PostgresError::from)
        })
        .await
    }

    pub async fn update_repository_member_permissions(
        &self,
        command: UpdateRepositoryMemberPermissionsCommand,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<RepositoryCollaborationMutation<RepositoryMember>, PostgresError> {
        let UpdateRepositoryMemberPermissionsCommand {
            owner,
            name,
            owner_user_id,
            member_user_id,
            permissions,
            now_unix,
        } = command;
        mutate_repository_collaboration(self, &owner, &name, now_unix, generated_ids, move |repo| {
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
    ) -> Result<RepositoryCollaborationMutation<RepositoryInvite>, PostgresError> {
        let owner_user_id = owner_user_id.to_string();
        let invite_id = invite_id.to_string();
        mutate_repository_collaboration(self, owner, name, now_unix, generated_ids, move |repo| {
            revoke_repository_invite(repo, &owner_user_id, &invite_id, now_unix)
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
    ) -> Result<RepositoryCollaborationMutation<RepositoryMember>, PostgresError> {
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
        save_repo_mutation(
            &tx,
            &before,
            &repo,
            &mutation_effects_none(),
            now_unix,
            generated_ids,
        )
        .await?;
        super::request_attention::remove_member_attention(&tx, &repo_id, member_user_id).await?;
        super::request_auto_merge::stop_auto_merges_for_revoked_actor(
            &tx,
            &repo_id,
            member_user_id,
            now_unix,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RepositoryCollaborationMutation::committed(&repo, removed))
    }

    /// `None` when no invite owns the link, which the landing page reports as
    /// a link that does not work rather than as an error.
    pub async fn repository_invite_by_link_hash(
        &self,
        link_hash: &str,
    ) -> Result<Option<(Repository, RepositoryInvite)>, PostgresError> {
        let Some(repo_id) = repo_id_for_invite_link(self.db.as_ref(), link_hash).await? else {
            return Ok(None);
        };
        let repo_row = entities::repository::Entity::find_by_id(repo_id)
            .one(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("repository invite repo is missing"))?;
        let repo = repository_from_model(self.db.as_ref(), repo_row).await?;
        let invite = repo
            .invitations
            .iter()
            .find(|invite| invite.link_hashes.iter().any(|hash| hash == link_hash))
            .cloned();
        Ok(invite.map(|invite| (repo, invite)))
    }

    pub async fn accept_repository_invite(
        &self,
        link_hash: &str,
        user: UserAccount,
        now_unix: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<(Repository, AcceptRepositoryInviteOutcome), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let repo_id = repo_id_for_invite_link(&tx, link_hash)
            .await?
            .ok_or_else(|| PostgresError::not_found("repository invite not found"))?;
        // The repository lock orders acceptance against revocation, member
        // removal, and a second acceptance of the same invite.
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        let row = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("repository invite not found"))?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        let outcome = accept_repository_invite(&mut repo, &user, link_hash, now_unix)?;
        if matches!(outcome, AcceptRepositoryInviteOutcome::Accepted(_)) {
            save_repo_mutation(
                &tx,
                &before,
                &repo,
                &mutation_effects_none(),
                now_unix,
                generated_ids,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok((repo, outcome))
    }
}

async fn repo_id_for_invite_link<C>(
    conn: &C,
    link_hash: &str,
) -> Result<Option<String>, PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let Some(link) = entities::repository_invite_link::Entity::find_by_id(link_hash.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(None);
    };
    Ok(
        entities::repository_invite::Entity::find_by_id(link.invite_id)
            .one(conn)
            .await
            .map_err(PostgresError::internal)?
            .map(|invite| invite.repo_id),
    )
}

async fn mutate_repository_collaboration<T, F>(
    store: &RepositoryStore,
    owner: &str,
    name: &str,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
    op: F,
) -> Result<RepositoryCollaborationMutation<T>, PostgresError>
where
    T: Send + 'static,
    F: FnOnce(&mut Repository) -> Result<T, PostgresError> + Send + 'static,
{
    let repo_id = repo_id(owner, name);
    let owner = owner.to_string();
    let name = name.to_string();
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
    save_repo_mutation(
        &tx,
        &before,
        &repo,
        &mutation_effects_none(),
        now_unix,
        generated_ids,
    )
    .await?;
    tx.commit().await.map_err(PostgresError::internal)?;
    Ok(RepositoryCollaborationMutation::committed(&repo, result))
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

fn mutation_effects_none() -> scope_domain::repo_actions::RepoEffects {
    scope_domain::repo_actions::RepoEffects::default()
}
