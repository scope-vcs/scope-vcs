use super::{
    RepositoryStore,
    auth::load_user_by_id,
    collaboration_rows::{
        load_collaboration_state, load_repository_collaboration, lock_collaboration_state,
        save_collaboration_state,
    },
    entities,
    repo_invite_emails::{latest_invite_emails, queue_invite_email},
};
use crate::error::{PostgresError, PostgresErrorKind};
use scope_domain::{
    account::UserAccount,
    error::DomainError,
    repo_collaboration::{
        AcceptRepositoryInviteOutcome, CreateRepositoryInviteCommand, accept_repository_invite,
        create_repository_invite, issue_repository_invite_link, remove_repository_member,
        revoke_repository_invite, update_repository_member_permissions,
    },
    repo_invite_email::RepositoryInviteEmail,
    repository::collaboration::{
        CollaborationState, RepositoryCollaboration, RepositoryInvite, RepositoryMember,
        RepositoryMemberPermissions, normalize_repository_invite_email,
    },
    repository::{RepoRecord, RepositoryIncarnation, access::RepositoryAccessContext, repo_id},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use std::collections::BTreeMap;

pub struct RepositoryCollaborationMutation<T> {
    pub incarnation: RepositoryIncarnation,
    pub change_version: u64,
    pub value: T,
}

impl<T> RepositoryCollaborationMutation<T> {
    pub(super) fn committed(record: &RepoRecord, value: T) -> Self {
        Self {
            incarnation: record.incarnation(),
            change_version: record.change_version,
            value,
        }
    }
}

/// What the owner's members list shows.
pub struct RepositoryCollaborationRead {
    pub collaboration: RepositoryCollaboration,
    pub users: BTreeMap<String, UserAccount>,
    /// The newest email of each invite, by invite id.
    pub invite_emails: BTreeMap<String, RepositoryInviteEmail>,
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
    /// The members and invites an owner manages, read in the same snapshot
    /// that authorized the viewer. `None` when the viewer cannot see the
    /// repository; a forbidden error when they can but do not own it.
    pub async fn repository_collaboration(
        &self,
        owner: &str,
        name: &str,
        viewer_user_id: &str,
    ) -> Result<Option<RepositoryCollaborationRead>, PostgresError> {
        let Some((tx, context)) = self
            .begin_read_access_snapshot(&repo_id(owner, name), Some(viewer_user_id))
            .await?
        else {
            return Ok(None);
        };
        context.ensure_owner()?;
        let collaboration = load_repository_collaboration(&tx, &context.record.id).await?;
        let user_ids = collaboration
            .members
            .iter()
            .map(|member| member.user_id.clone())
            .collect::<Vec<_>>();
        let users = if user_ids.is_empty() {
            BTreeMap::new()
        } else {
            entities::user::Entity::find()
                .filter(entities::user::Column::Id.is_in(user_ids))
                .all(&tx)
                .await
                .map_err(PostgresError::internal)?
                .into_iter()
                .map(|row| {
                    let user = row.try_into_domain()?;
                    Ok((user.id.clone(), user))
                })
                .collect::<Result<_, PostgresError>>()?
        };
        let invite_emails = latest_invite_emails(&tx, &collaboration.invitations).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(RepositoryCollaborationRead {
            collaboration,
            users,
            invite_emails,
        }))
    }

    pub async fn user(&self, user_id: &str) -> Result<UserAccount, PostgresError> {
        load_user_by_id(self.db.as_ref(), user_id).await
    }

    pub async fn create_repository_invite(
        &self,
        command: CreateRepositoryInviteMutation,
    ) -> Result<
        RepositoryCollaborationMutation<(RepositoryInvite, Option<RepositoryInviteEmail>)>,
        PostgresError,
    > {
        let now_unix = command.now_unix;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let mut repo = lock_owned_collaboration(&tx, &command.owner, &command.name).await?;
        let before = repo.clone();
        let invitee = user_by_normalized_email(&tx, &command.invited_email).await?;
        let invite = create_repository_invite(
            &mut repo,
            CreateRepositoryInviteCommand {
                id: command.invite_id,
                owner: &command.owner_user,
                invited_email: command.invited_email,
                invitee: invitee.as_ref(),
                permissions: command.permissions,
                now_unix,
            },
        )?;
        save_collaboration_state(&tx, &before, &repo).await?;
        // The invite and its first email commit together, so an invite is
        // never saved with a forgotten email. An owner who has used up the
        // daily email allowance still gets the invite, and can copy a link.
        let email = match queue_invite_email(
            &tx,
            &repo,
            &command.owner_user.id,
            &invite.id,
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
            &repo.record,
            (invite, email),
        ))
    }

    pub async fn issue_repository_invite_link(
        &self,
        command: IssueRepositoryInviteLinkCommand,
    ) -> Result<RepositoryCollaborationMutation<RepositoryInvite>, PostgresError> {
        let IssueRepositoryInviteLinkCommand {
            owner,
            name,
            owner_user_id,
            invite_id,
            link_hash,
            now_unix,
        } = command;
        mutate_collaboration(self, &owner, &name, |repo| {
            issue_repository_invite_link(repo, &owner_user_id, &invite_id, link_hash, now_unix)
        })
        .await
    }

    pub async fn update_repository_member_permissions(
        &self,
        command: UpdateRepositoryMemberPermissionsCommand,
    ) -> Result<RepositoryCollaborationMutation<RepositoryMember>, PostgresError> {
        let UpdateRepositoryMemberPermissionsCommand {
            owner,
            name,
            owner_user_id,
            member_user_id,
            permissions,
            now_unix,
        } = command;
        mutate_collaboration(self, &owner, &name, |repo| {
            update_repository_member_permissions(
                repo,
                &owner_user_id,
                &member_user_id,
                permissions,
                now_unix,
            )
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
    ) -> Result<RepositoryCollaborationMutation<RepositoryInvite>, PostgresError> {
        mutate_collaboration(self, owner, name, |repo| {
            revoke_repository_invite(repo, owner_user_id, invite_id, now_unix)
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
    ) -> Result<RepositoryCollaborationMutation<RepositoryMember>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let mut repo = lock_owned_collaboration(&tx, owner, name).await?;
        let before = repo.clone();
        let removed = remove_repository_member(&mut repo, owner_user_id, member_user_id)?;
        save_collaboration_state(&tx, &before, &repo).await?;
        let repo_id = &repo.record.id;
        super::request_attention::remove_member_attention(&tx, repo_id, member_user_id).await?;
        super::request_auto_merge::stop_auto_merges_for_revoked_actor(
            &tx,
            repo_id,
            member_user_id,
            now_unix,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(RepositoryCollaborationMutation::committed(
            &repo.record,
            removed,
        ))
    }

    /// `None` when no invite owns the link, which the landing page reports as
    /// a link that does not work rather than as an error.
    pub async fn repository_invite_by_link_hash(
        &self,
        link_hash: &str,
    ) -> Result<Option<(CollaborationState, RepositoryInvite)>, PostgresError> {
        let Some(repo_id) = repo_id_for_invite_link(self.db.as_ref(), link_hash).await? else {
            return Ok(None);
        };
        let repo = load_collaboration_state(self.db.as_ref(), &repo_id)
            .await?
            .ok_or_else(|| PostgresError::internal_message("repository invite repo is missing"))?;
        let invite = repo
            .collaboration
            .invitations
            .iter()
            .find(|invite| invite.link_hashes.iter().any(|hash| hash == link_hash))
            .cloned();
        Ok(invite.map(|invite| (repo, invite)))
    }

    /// Returns the repository as the accepting user now sees it.
    pub async fn accept_repository_invite(
        &self,
        link_hash: &str,
        user: UserAccount,
        now_unix: u64,
    ) -> Result<(RepositoryAccessContext, AcceptRepositoryInviteOutcome), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let repo_id = repo_id_for_invite_link(&tx, link_hash)
            .await?
            .ok_or_else(|| PostgresError::not_found("repository invite not found"))?;
        // The repository lock orders acceptance against revocation, member
        // removal, and a second acceptance of the same invite.
        let mut repo = lock_collaboration_state(&tx, &repo_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("repository invite not found"))?;
        let before = repo.clone();
        let outcome = accept_repository_invite(&mut repo, &user, link_hash, now_unix)?;
        if matches!(outcome, AcceptRepositoryInviteOutcome::Accepted(_)) {
            save_collaboration_state(&tx, &before, &repo).await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        let access = repo.access_for_user_id(&user.id);
        Ok((
            RepositoryAccessContext {
                record: repo.record,
                access,
            },
            outcome,
        ))
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

async fn lock_owned_collaboration(
    tx: &sea_orm::DatabaseTransaction,
    owner: &str,
    name: &str,
) -> Result<CollaborationState, PostgresError> {
    lock_collaboration_state(tx, &repo_id(owner, name))
        .await?
        .ok_or_else(|| PostgresError::not_found(format!("repo {owner}/{name} not found")))
}

async fn mutate_collaboration<T>(
    store: &RepositoryStore,
    owner: &str,
    name: &str,
    op: impl FnOnce(&mut CollaborationState) -> Result<T, DomainError>,
) -> Result<RepositoryCollaborationMutation<T>, PostgresError> {
    let tx = store.db.begin().await.map_err(PostgresError::internal)?;
    let mut repo = lock_owned_collaboration(&tx, owner, name).await?;
    let before = repo.clone();
    let value = op(&mut repo)?;
    save_collaboration_state(&tx, &before, &repo).await?;
    tx.commit().await.map_err(PostgresError::internal)?;
    Ok(RepositoryCollaborationMutation::committed(
        &repo.record,
        value,
    ))
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
