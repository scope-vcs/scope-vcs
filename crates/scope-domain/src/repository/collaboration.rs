use super::{
    RepoRecord, RepositoryIncarnation,
    access::{RepositoryAccess, repository_access_for_user_id},
};
use crate::{
    error::DomainError,
    views::{ViewId, Views},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryMemberPermissions {
    pub can_push: bool,
    pub can_change_file_visibility: bool,
    pub view: ViewId,
}

impl RepositoryMemberPermissions {
    pub fn validate(&self, views: &Views) -> Result<(), DomainError> {
        if views.get(&self.view).is_none() {
            return Err(DomainError::invalid_input(format!(
                "unknown view {}",
                self.view
            )));
        }
        if self.can_change_file_visibility && &self.view != views.full() {
            return Err(DomainError::invalid_input(format!(
                "members who change file visibility are assigned the {} view",
                views.display_name(views.full())
            )));
        }
        Ok(())
    }
}

impl Default for RepositoryMemberPermissions {
    fn default() -> Self {
        Self {
            can_push: false,
            can_change_file_visibility: false,
            view: ViewId::private(),
        }
    }
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
    pub link_hashes: Vec<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub expires_at_unix: u64,
    pub accepted_by_user_id: Option<String>,
    pub accepted_at_unix: Option<u64>,
    pub revoked_at_unix: Option<u64>,
}

impl RepositoryInvite {
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

    pub fn ended_at_unix(&self) -> u64 {
        self.revoked_at_unix
            .or(self.accepted_at_unix)
            .unwrap_or(self.expires_at_unix)
    }
}

pub fn normalize_repository_invite_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryCollaboration {
    pub members: Vec<RepositoryMember>,
    pub invitations: Vec<RepositoryInvite>,
}

impl RepositoryCollaboration {
    pub fn member_for_user(&self, user_id: &str) -> Option<&RepositoryMember> {
        self.members.iter().find(|member| member.user_id == user_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollaborationState {
    pub record: RepoRecord,
    pub views: Views,
    pub collaboration: RepositoryCollaboration,
}

impl CollaborationState {
    pub fn is_owner_user(&self, user_id: &str) -> bool {
        self.record.owner_user_id == user_id
    }

    pub fn incarnation(&self) -> RepositoryIncarnation {
        self.record.incarnation()
    }

    pub fn member_for_user(&self, user_id: &str) -> Option<&RepositoryMember> {
        self.collaboration.member_for_user(user_id)
    }

    pub fn access_for_user_id(&self, user_id: &str) -> RepositoryAccess {
        repository_access_for_user_id(
            &self.record.owner_user_id,
            self.record.lifecycle_state,
            self.member_for_user(user_id)
                .map(|member| member.permissions.clone()),
            user_id,
        )
    }
}
