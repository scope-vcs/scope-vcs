use super::{
    account::UserAccount,
    repository::collaboration::{
        CollaborationState, RepositoryInvite, RepositoryInviteState, RepositoryMember,
        RepositoryMemberPermissions, normalize_repository_invite_email,
    },
};
use crate::error::DomainError;

pub const REPOSITORY_INVITE_TTL_SECS: u64 = 7 * 24 * 60 * 60;
pub const REPOSITORY_INVITE_RETENTION_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryInviteLanding {
    Open(RepositoryInviteViewer),
    Member,
    Expired,
    Revoked,
    AccessRemoved,
    Used,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryInviteViewer {
    Ready,
    SignedOut,
    WrongAccount,
    EmailUnverified,
}

pub enum AcceptRepositoryInviteOutcome {
    Accepted(RepositoryMember),
    AlreadyAccepted(RepositoryMember),
}

pub struct CreateRepositoryInviteCommand<'a> {
    pub id: String,
    pub owner: &'a UserAccount,
    pub invited_email: String,
    pub invitee: Option<&'a UserAccount>,
    pub permissions: RepositoryMemberPermissions,
    pub now_unix: u64,
}

pub fn create_repository_invite(
    repo: &mut CollaborationState,
    command: CreateRepositoryInviteCommand<'_>,
) -> Result<RepositoryInvite, DomainError> {
    ensure_can_manage_members(repo, &command.owner.id)?;
    let normalized = validate_invite_email(&command.invited_email)?;
    if normalize_repository_invite_email(&command.owner.email) == normalized {
        return Err(DomainError::conflict("repository owner cannot be invited"));
    }
    if let Some(invitee) = command.invitee
        && (repo.is_owner_user(&invitee.id) || repo.member_for_user(&invitee.id).is_some())
    {
        return Err(DomainError::conflict("user is already a repository member"));
    }
    if repo.collaboration.invitations.iter().any(|invite| {
        invite.invited_email_normalized == normalized
            && invite.state(command.now_unix) == RepositoryInviteState::Pending
    }) {
        return Err(DomainError::conflict(
            "this email already has a pending invite; resend it, copy a link, or revoke it",
        ));
    }

    command.permissions.validate(&repo.views)?;
    let invite = RepositoryInvite {
        id: command.id,
        repo_id: repo.record.id.clone(),
        invited_email: command.invited_email.trim().to_string(),
        invited_email_normalized: normalized,
        permissions: command.permissions,
        invited_by_user_id: command.owner.id.clone(),
        link_hashes: Vec::new(),
        created_at_unix: command.now_unix,
        updated_at_unix: command.now_unix,
        expires_at_unix: command.now_unix + REPOSITORY_INVITE_TTL_SECS,
        accepted_by_user_id: None,
        accepted_at_unix: None,
        revoked_at_unix: None,
    };
    repo.collaboration.invitations.push(invite.clone());
    sort_invitations(repo);
    repo.record.bump_change_version();
    Ok(invite)
}

pub fn issue_repository_invite_link(
    repo: &mut CollaborationState,
    owner_user_id: &str,
    invite_id: &str,
    link_hash: String,
    now_unix: u64,
) -> Result<RepositoryInvite, DomainError> {
    ensure_can_manage_members(repo, owner_user_id)?;
    let invite = pending_invite_mut(repo, invite_id, now_unix)?;
    invite.link_hashes.push(link_hash);
    invite.updated_at_unix = now_unix;
    let invite = invite.clone();
    repo.record.bump_change_version();
    Ok(invite)
}

pub fn repository_invite_landing(
    repo: &CollaborationState,
    invite: &RepositoryInvite,
    viewer: Option<&UserAccount>,
    now_unix: u64,
) -> RepositoryInviteLanding {
    let viewer_has_access = viewer.is_some_and(|viewer| {
        repo.is_owner_user(&viewer.id) || repo.member_for_user(&viewer.id).is_some()
    });
    match invite.state(now_unix) {
        RepositoryInviteState::Revoked => RepositoryInviteLanding::Revoked,
        RepositoryInviteState::Expired => RepositoryInviteLanding::Expired,
        RepositoryInviteState::Accepted => {
            let accepted_by_viewer = viewer
                .is_some_and(|viewer| invite.accepted_by_user_id.as_deref() == Some(&viewer.id));
            match (accepted_by_viewer, viewer_has_access) {
                (_, true) => RepositoryInviteLanding::Member,
                (true, false) => RepositoryInviteLanding::AccessRemoved,
                (false, false) => RepositoryInviteLanding::Used,
            }
        }
        RepositoryInviteState::Pending if viewer_has_access => RepositoryInviteLanding::Member,
        RepositoryInviteState::Pending => RepositoryInviteLanding::Open(match viewer {
            None => RepositoryInviteViewer::SignedOut,
            Some(viewer)
                if normalize_repository_invite_email(&viewer.email)
                    != invite.invited_email_normalized =>
            {
                RepositoryInviteViewer::WrongAccount
            }
            Some(viewer) if !viewer.email_verified => RepositoryInviteViewer::EmailUnverified,
            Some(_) => RepositoryInviteViewer::Ready,
        }),
    }
}

pub fn accept_repository_invite(
    repo: &mut CollaborationState,
    user: &UserAccount,
    link_hash: &str,
    now_unix: u64,
) -> Result<AcceptRepositoryInviteOutcome, DomainError> {
    let index = repo
        .collaboration
        .invitations
        .iter()
        .position(|invite| invite.link_hashes.iter().any(|hash| hash == link_hash))
        .ok_or_else(|| DomainError::not_found("repository invite not found"))?;
    match repository_invite_landing(
        repo,
        &repo.collaboration.invitations[index],
        Some(user),
        now_unix,
    ) {
        RepositoryInviteLanding::Open(RepositoryInviteViewer::Ready) => {}
        RepositoryInviteLanding::Open(_) => {
            return Err(DomainError::forbidden(
                "sign in with the verified invited email to accept this invite",
            ));
        }
        RepositoryInviteLanding::Member => {
            let accepted_here = repo.collaboration.invitations[index]
                .accepted_by_user_id
                .as_deref()
                == Some(&user.id);
            return match repo.member_for_user(&user.id) {
                Some(member) if accepted_here => Ok(
                    AcceptRepositoryInviteOutcome::AlreadyAccepted(member.clone()),
                ),
                _ => Err(DomainError::conflict("user is already a repository member")),
            };
        }
        RepositoryInviteLanding::Expired => {
            return Err(DomainError::conflict("repository invite expired"));
        }
        RepositoryInviteLanding::Revoked => {
            return Err(DomainError::conflict("repository invite was revoked"));
        }
        RepositoryInviteLanding::Used => {
            return Err(DomainError::conflict("repository invite was already used"));
        }
        RepositoryInviteLanding::AccessRemoved => {
            return Err(DomainError::forbidden(
                "repository access was removed; ask the owner for a new invite",
            ));
        }
    }

    if repo.collaboration.invitations[index]
        .permissions
        .validate(&repo.views)
        .is_err()
    {
        return Err(DomainError::conflict(
            "the invited view no longer exists; ask the owner for a new invite",
        ));
    }
    let invite = &mut repo.collaboration.invitations[index];
    invite.accepted_by_user_id = Some(user.id.clone());
    invite.accepted_at_unix = Some(now_unix);
    invite.updated_at_unix = now_unix;
    let member = RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: user.id.clone(),
        permissions: invite.permissions.clone(),
        created_at_unix: now_unix,
        updated_at_unix: now_unix,
    };
    repo.collaboration.members.push(member.clone());
    sort_members(repo);
    repo.record.bump_change_version();
    Ok(AcceptRepositoryInviteOutcome::Accepted(member))
}

pub fn revoke_repository_invite(
    repo: &mut CollaborationState,
    owner_user_id: &str,
    invite_id: &str,
    now_unix: u64,
) -> Result<RepositoryInvite, DomainError> {
    ensure_can_manage_members(repo, owner_user_id)?;
    let invite = pending_invite_mut(repo, invite_id, now_unix)?;
    invite.revoked_at_unix = Some(now_unix);
    invite.updated_at_unix = now_unix;
    let invite = invite.clone();
    repo.record.bump_change_version();
    Ok(invite)
}

pub fn prune_ended_repository_invites(
    repo: &mut CollaborationState,
    now_unix: u64,
) -> Vec<RepositoryInvite> {
    let (pruned, kept) = std::mem::take(&mut repo.collaboration.invitations)
        .into_iter()
        .partition::<Vec<_>, _>(|invite| {
            invite
                .ended_at_unix()
                .saturating_add(REPOSITORY_INVITE_RETENTION_SECS)
                <= now_unix
        });
    repo.collaboration.invitations = kept;
    if !pruned.is_empty() {
        repo.record.bump_change_version();
    }
    pruned
}

fn pending_invite_mut<'a>(
    repo: &'a mut CollaborationState,
    invite_id: &str,
    now_unix: u64,
) -> Result<&'a mut RepositoryInvite, DomainError> {
    let invite = repo
        .collaboration
        .invitations
        .iter_mut()
        .find(|invite| invite.id == invite_id)
        .ok_or_else(|| DomainError::not_found("repository invite not found"))?;
    if invite.state(now_unix) != RepositoryInviteState::Pending {
        return Err(DomainError::conflict(
            "repository invite is no longer pending",
        ));
    }
    Ok(invite)
}

pub fn update_repository_member_permissions(
    repo: &mut CollaborationState,
    owner_user_id: &str,
    member_user_id: &str,
    permissions: RepositoryMemberPermissions,
    now_unix: u64,
) -> Result<RepositoryMember, DomainError> {
    ensure_can_manage_members(repo, owner_user_id)?;
    permissions.validate(&repo.views)?;
    let member = repo
        .collaboration
        .members
        .iter_mut()
        .find(|member| member.user_id == member_user_id)
        .ok_or_else(|| DomainError::not_found("repository member not found"))?;
    member.permissions = permissions;
    member.updated_at_unix = now_unix;
    let member = member.clone();
    repo.record.bump_change_version();
    Ok(member)
}

pub fn remove_repository_member(
    repo: &mut CollaborationState,
    owner_user_id: &str,
    member_user_id: &str,
) -> Result<RepositoryMember, DomainError> {
    ensure_can_manage_members(repo, owner_user_id)?;
    let index = repo
        .collaboration
        .members
        .iter()
        .position(|member| member.user_id == member_user_id)
        .ok_or_else(|| DomainError::not_found("repository member not found"))?;
    let removed = repo.collaboration.members.remove(index);
    repo.record.bump_change_version();
    Ok(removed)
}

pub fn ensure_can_manage_members(
    repo: &CollaborationState,
    user_id: &str,
) -> Result<(), DomainError> {
    if repo.access_for_user_id(user_id).can_manage_members {
        Ok(())
    } else if repo.is_owner_user(user_id) {
        Err(DomainError::conflict(
            "repository must be ready before inviting members",
        ))
    } else {
        Err(DomainError::forbidden("owner role required"))
    }
}

fn validate_invite_email(email: &str) -> Result<String, DomainError> {
    let normalized = normalize_repository_invite_email(email);
    if normalized.is_empty() || !normalized.contains('@') {
        return Err(DomainError::invalid_input(
            "valid invited email is required",
        ));
    }
    Ok(normalized)
}

fn sort_members(repo: &mut CollaborationState) {
    repo.collaboration.members.sort_by(|left, right| {
        left.user_id
            .cmp(&right.user_id)
            .then(left.created_at_unix.cmp(&right.created_at_unix))
    });
}

fn sort_invitations(repo: &mut CollaborationState) {
    repo.collaboration.invitations.sort_by(|left, right| {
        left.invited_email_normalized
            .cmp(&right.invited_email_normalized)
            .then(left.id.cmp(&right.id))
    });
}
