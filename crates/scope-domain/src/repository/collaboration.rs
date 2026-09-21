use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryMemberPermissions {
    pub can_push: bool,
    pub can_change_file_visibility: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryMember {
    pub repo_id: String,
    pub user_id: String,
    pub permissions: RepositoryMemberPermissions,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryInviteState {
    Pending,
    Accepted,
    Revoked,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInvite {
    pub id: String,
    pub repo_id: String,
    pub invited_email: String,
    pub invited_email_normalized: String,
    pub permissions: RepositoryMemberPermissions,
    pub invited_by_user_id: String,
    /// Every link issued for this invite. All of them work until the invite
    /// stops being pending.
    pub link_hashes: Vec<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub expires_at_unix: u64,
    pub accepted_by_user_id: Option<String>,
    pub accepted_at_unix: Option<u64>,
    pub revoked_at_unix: Option<u64>,
}

impl RepositoryInvite {
    /// The state is never stored, so the members list, the landing page, and
    /// acceptance cannot disagree about whether an invite has expired.
    pub fn state(&self, now_unix: u64) -> RepositoryInviteState {
        if self.revoked_at_unix.is_some() {
            RepositoryInviteState::Revoked
        } else if self.accepted_at_unix.is_some() {
            RepositoryInviteState::Accepted
        } else if now_unix >= self.expires_at_unix {
            RepositoryInviteState::Expired
        } else {
            RepositoryInviteState::Pending
        }
    }
}

pub fn normalize_repository_invite_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}
