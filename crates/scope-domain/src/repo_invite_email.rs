use super::{
    repo_collaboration::ensure_can_manage_members,
    repository::collaboration::{CollaborationState, RepositoryInviteState},
};
use crate::error::DomainError;

pub const INVITE_EMAIL_MIN_INTERVAL_SECS: u64 = 60;
pub const INVITE_EMAIL_MAX_PER_INVITE: usize = 5;
pub const INVITE_EMAIL_MAX_PER_OWNER_PER_DAY: usize = 20;
pub const INVITE_EMAIL_OWNER_WINDOW_SECS: u64 = 24 * 60 * 60;
pub const INVITE_EMAIL_MAX_ATTEMPTS: u32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryInviteEmailState {
    Queued,
    Sent,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryInviteEmail {
    pub id: String,
    pub invite_id: String,
    pub requested_by_user_id: String,
    pub state: RepositoryInviteEmailState,
    pub attempts: u32,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

impl RepositoryInviteEmail {
    fn counts_as_send(&self) -> bool {
        self.state != RepositoryInviteEmailState::Failed
    }
}

pub struct InviteEmailHistory<'a> {
    pub for_invite: &'a [RepositoryInviteEmail],
    pub owner_recent_sends_unix: &'a [u64],
}

pub struct RequestInviteEmailCommand<'a> {
    pub id: String,
    pub owner_user_id: &'a str,
    pub invite_id: &'a str,
    pub now_unix: u64,
}

pub fn request_repository_invite_email(
    repo: &CollaborationState,
    command: RequestInviteEmailCommand<'_>,
    history: InviteEmailHistory<'_>,
) -> Result<RepositoryInviteEmail, DomainError> {
    ensure_can_manage_members(repo, command.owner_user_id)?;
    let now = command.now_unix;
    let invite = repo
        .collaboration
        .invitations
        .iter()
        .find(|invite| invite.id == command.invite_id)
        .ok_or_else(|| DomainError::not_found("repository invite not found"))?;
    if invite.state(now) != RepositoryInviteState::Pending {
        return Err(DomainError::conflict(
            "repository invite is no longer pending",
        ));
    }
    if history
        .for_invite
        .iter()
        .any(|email| email.state == RepositoryInviteEmailState::Queued)
    {
        return Err(DomainError::conflict(
            "an email for this invite is already being sent",
        ));
    }

    if let Some(latest) = history
        .for_invite
        .iter()
        .map(|email| email.created_at_unix)
        .max()
    {
        let next_allowed = latest + INVITE_EMAIL_MIN_INTERVAL_SECS;
        if now < next_allowed {
            return Err(DomainError::rate_limited(format!(
                "this invite was emailed less than a minute ago; try again in {} seconds",
                next_allowed - now
            )));
        }
    }
    let sends_for_invite = history
        .for_invite
        .iter()
        .filter(|email| email.counts_as_send())
        .count();
    if sends_for_invite >= INVITE_EMAIL_MAX_PER_INVITE {
        return Err(DomainError::rate_limited(format!(
            "this invite has used all {INVITE_EMAIL_MAX_PER_INVITE} of its emails; copy a link instead"
        )));
    }
    if history.owner_recent_sends_unix.len() >= INVITE_EMAIL_MAX_PER_OWNER_PER_DAY {
        let oldest = history
            .owner_recent_sends_unix
            .iter()
            .copied()
            .min()
            .unwrap_or(now);
        let minutes = (oldest + INVITE_EMAIL_OWNER_WINDOW_SECS)
            .saturating_sub(now)
            .div_ceil(60);
        return Err(DomainError::rate_limited(format!(
            "you have sent {INVITE_EMAIL_MAX_PER_OWNER_PER_DAY} invite emails in the last day; \
             try again in {minutes} minutes or copy a link instead"
        )));
    }

    Ok(RepositoryInviteEmail {
        id: command.id,
        invite_id: invite.id.clone(),
        requested_by_user_id: command.owner_user_id.to_string(),
        state: RepositoryInviteEmailState::Queued,
        attempts: 0,
        created_at_unix: now,
        updated_at_unix: now,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InviteEmailAttempt {
    Accepted,
    Retryable(String),
    Refused(String),
}

pub fn record_invite_email_attempt(
    email: &mut RepositoryInviteEmail,
    attempt: &InviteEmailAttempt,
    now_unix: u64,
) -> Option<u64> {
    email.attempts += 1;
    email.updated_at_unix = now_unix;
    match attempt {
        InviteEmailAttempt::Accepted => {
            email.state = RepositoryInviteEmailState::Sent;
            None
        }
        InviteEmailAttempt::Retryable(_) if email.attempts < INVITE_EMAIL_MAX_ATTEMPTS => {
            Some(now_unix + 30 * (1 << email.attempts.min(6)))
        }
        InviteEmailAttempt::Retryable(_) | InviteEmailAttempt::Refused(_) => {
            email.state = RepositoryInviteEmailState::Failed;
            None
        }
    }
}
