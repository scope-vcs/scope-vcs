use super::{
    account::UserAccount,
    repository::Repository,
    repository::collaboration::{
        RepositoryInvite, RepositoryInviteState, RepositoryMember, RepositoryMemberPermissions,
        normalize_repository_invite_email,
    },
};
use crate::error::DomainError;

pub const REPOSITORY_INVITE_TTL_SECS: u64 = 7 * 24 * 60 * 60;

/// What the person opening an invite link should be shown, and therefore what
/// they are allowed to do with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryInviteLanding {
    Open(RepositoryInviteViewer),
    /// The viewer already has access, through this invite or another way.
    Member,
    Expired,
    Revoked,
    /// The viewer accepted this invite and was later removed.
    AccessRemoved,
    /// Someone else accepted this invite, and the viewer has no access.
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
    /// A repeat of an acceptance that already succeeded, such as a double
    /// click or a retry after a lost response.
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
    repo: &mut Repository,
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
    if repo.invitations.iter().any(|invite| {
        invite.invited_email_normalized == normalized
            && invite.state(command.now_unix) == RepositoryInviteState::Pending
    }) {
        return Err(DomainError::conflict(
            "this email already has a pending invite; resend it, copy a link, or revoke it",
        ));
    }

    let invite = RepositoryInvite {
        id: command.id,
        repo_id: repo.record.id.clone(),
        invited_email: command.invited_email.trim().to_string(),
        invited_email_normalized: normalized,
        permissions: command.permissions,
        invited_by_user_id: command.owner.id.clone(),
        // The sender issues a link when it emails the invite, and the owner
        // can copy one. Nothing stores a link that was never handed out.
        link_hashes: Vec::new(),
        created_at_unix: command.now_unix,
        updated_at_unix: command.now_unix,
        expires_at_unix: command.now_unix + REPOSITORY_INVITE_TTL_SECS,
        accepted_by_user_id: None,
        accepted_at_unix: None,
        revoked_at_unix: None,
    };
    repo.invitations.push(invite.clone());
    sort_invitations(repo);
    repo.bump_change_version();
    Ok(invite)
}

/// Adds one more working link. Earlier links and the expiry stay as they are.
pub fn issue_repository_invite_link(
    repo: &mut Repository,
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
    repo.bump_change_version();
    Ok(invite)
}

pub fn repository_invite_landing(
    repo: &Repository,
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
    repo: &mut Repository,
    user: &UserAccount,
    link_hash: &str,
    now_unix: u64,
) -> Result<AcceptRepositoryInviteOutcome, DomainError> {
    let index = repo
        .invitations
        .iter()
        .position(|invite| invite.link_hashes.iter().any(|hash| hash == link_hash))
        .ok_or_else(|| DomainError::not_found("repository invite not found"))?;
    match repository_invite_landing(repo, &repo.invitations[index], Some(user), now_unix) {
        RepositoryInviteLanding::Open(RepositoryInviteViewer::Ready) => {}
        RepositoryInviteLanding::Open(_) => {
            return Err(DomainError::forbidden(
                "sign in with the verified invited email to accept this invite",
            ));
        }
        RepositoryInviteLanding::Member => {
            let accepted_here =
                repo.invitations[index].accepted_by_user_id.as_deref() == Some(&user.id);
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

    let invite = &mut repo.invitations[index];
    invite.accepted_by_user_id = Some(user.id.clone());
    invite.accepted_at_unix = Some(now_unix);
    invite.updated_at_unix = now_unix;
    let member = RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: user.id.clone(),
        permissions: invite.permissions,
        created_at_unix: now_unix,
        updated_at_unix: now_unix,
    };
    repo.members.push(member.clone());
    sort_members(repo);
    repo.bump_change_version();
    Ok(AcceptRepositoryInviteOutcome::Accepted(member))
}

pub fn revoke_repository_invite(
    repo: &mut Repository,
    owner_user_id: &str,
    invite_id: &str,
    now_unix: u64,
) -> Result<RepositoryInvite, DomainError> {
    ensure_can_manage_members(repo, owner_user_id)?;
    let invite = pending_invite_mut(repo, invite_id, now_unix)?;
    invite.revoked_at_unix = Some(now_unix);
    invite.updated_at_unix = now_unix;
    let invite = invite.clone();
    repo.bump_change_version();
    Ok(invite)
}

fn pending_invite_mut<'a>(
    repo: &'a mut Repository,
    invite_id: &str,
    now_unix: u64,
) -> Result<&'a mut RepositoryInvite, DomainError> {
    let invite = repo
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
    repo: &mut Repository,
    owner_user_id: &str,
    member_user_id: &str,
    permissions: RepositoryMemberPermissions,
    now_unix: u64,
) -> Result<RepositoryMember, DomainError> {
    ensure_can_manage_members(repo, owner_user_id)?;
    let member = repo
        .members
        .iter_mut()
        .find(|member| member.user_id == member_user_id)
        .ok_or_else(|| DomainError::not_found("repository member not found"))?;
    member.permissions = permissions;
    member.updated_at_unix = now_unix;
    let member = member.clone();
    repo.bump_change_version();
    Ok(member)
}

pub fn remove_repository_member(
    repo: &mut Repository,
    owner_user_id: &str,
    member_user_id: &str,
) -> Result<RepositoryMember, DomainError> {
    ensure_can_manage_members(repo, owner_user_id)?;
    let index = repo
        .members
        .iter()
        .position(|member| member.user_id == member_user_id)
        .ok_or_else(|| DomainError::not_found("repository member not found"))?;
    let removed = repo.members.remove(index);
    repo.bump_change_version();
    Ok(removed)
}

pub fn ensure_can_manage_members(repo: &Repository, user_id: &str) -> Result<(), DomainError> {
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

fn sort_members(repo: &mut Repository) {
    repo.members.sort_by(|left, right| {
        left.user_id
            .cmp(&right.user_id)
            .then(left.created_at_unix.cmp(&right.created_at_unix))
    });
}

fn sort_invitations(repo: &mut Repository) {
    repo.invitations.sort_by(|left, right| {
        left.invited_email_normalized
            .cmp(&right.invited_email_normalized)
            .then(left.id.cmp(&right.id))
    });
}
