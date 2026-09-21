use super::RepoSummaryResponse;
use scope_api_contract::{RepositoryInviteState, RepositoryMemberPermissions};
use scope_domain::{
    account::UserAccount,
    repo_collaboration::{RepositoryInviteLanding, RepositoryInviteViewer},
    repository::Repository,
    repository::collaboration::{RepositoryInvite, RepositoryMember},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct RepositoryCollaborationResponse {
    pub(crate) members: Vec<RepositoryMemberResponse>,
    pub(crate) invites: Vec<RepositoryInviteResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct RepositoryMemberResponse {
    pub(crate) user_id: String,
    pub(crate) handle: String,
    pub(crate) email: String,
    pub(crate) permissions: RepositoryMemberPermissions,
    pub(crate) created_at_unix: u64,
    pub(crate) updated_at_unix: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct RepositoryInviteResponse {
    pub(crate) id: String,
    pub(crate) invited_email: String,
    pub(crate) permissions: RepositoryMemberPermissions,
    pub(crate) state: RepositoryInviteState,
    pub(crate) expires_at_unix: u64,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct CreateRepositoryInviteRequest {
    pub(crate) email: String,
    pub(crate) permissions: RepositoryMemberPermissions,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct CreateRepositoryInviteResponse {
    pub(crate) invite: RepositoryInviteResponse,
    pub(crate) invite_url: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct UpdateRepositoryMemberRequest {
    pub(crate) permissions: RepositoryMemberPermissions,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct RepositoryInviteLinkResponse {
    pub(crate) invite_url: String,
}

/// What an invite link shows its viewer. Only the repository owner can invite,
/// so `owner_handle` is also the inviter. Only an open link names the invited
/// email; a link someone else used, a revoked link, and an unknown link name
/// nothing.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(tag = "status", rename_all = "snake_case"))]
pub(crate) enum RepositoryInviteLandingResponse {
    Open {
        viewer: RepositoryInviteViewerResponse,
        viewer_email: Option<String>,
        owner_handle: String,
        repo_name: String,
        invited_email: String,
        permissions: RepositoryMemberPermissions,
        expires_at_unix: u64,
    },
    Member {
        owner_handle: String,
        repo_name: String,
    },
    Expired {
        owner_handle: String,
        repo_name: String,
    },
    Revoked,
    AccessRemoved,
    Used,
    Invalid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(rename_all = "snake_case"))]
pub(crate) enum RepositoryInviteViewerResponse {
    Ready,
    SignedOut,
    WrongAccount,
    EmailUnverified,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct AcceptRepositoryInviteResponse {
    pub(crate) repo: RepoSummaryResponse,
    pub(crate) member: RepositoryMemberResponse,
}

pub(crate) fn repository_collaboration_response(
    repo: &Repository,
    users: &std::collections::BTreeMap<String, UserAccount>,
    now_unix: u64,
) -> RepositoryCollaborationResponse {
    let mut members = repo
        .members
        .iter()
        .filter_map(|member| {
            users
                .get(&member.user_id)
                .map(|user| repository_member_response(member, user))
        })
        .collect::<Vec<_>>();
    members.sort_by(|left, right| {
        left.email
            .cmp(&right.email)
            .then(left.user_id.cmp(&right.user_id))
    });

    let mut invites = repo
        .invitations
        .iter()
        .map(|invite| repository_invite_response(invite, now_unix))
        .collect::<Vec<_>>();
    invites.sort_by(|left, right| {
        left.invited_email
            .cmp(&right.invited_email)
            .then(left.id.cmp(&right.id))
    });

    RepositoryCollaborationResponse { members, invites }
}

pub(crate) fn repository_member_response(
    member: &RepositoryMember,
    user: &UserAccount,
) -> RepositoryMemberResponse {
    RepositoryMemberResponse {
        user_id: member.user_id.clone(),
        handle: user.handle.clone(),
        email: user.email.clone(),
        permissions: member.permissions.into(),
        created_at_unix: member.created_at_unix,
        updated_at_unix: member.updated_at_unix,
    }
}

pub(crate) fn repository_invite_response(
    invite: &RepositoryInvite,
    now_unix: u64,
) -> RepositoryInviteResponse {
    RepositoryInviteResponse {
        id: invite.id.clone(),
        invited_email: invite.invited_email.clone(),
        permissions: invite.permissions.into(),
        state: invite.state(now_unix).into(),
        expires_at_unix: invite.expires_at_unix,
    }
}

pub(crate) fn repository_invite_landing_response(
    landing: RepositoryInviteLanding,
    repo: &Repository,
    invite: &RepositoryInvite,
    viewer: Option<&UserAccount>,
) -> RepositoryInviteLandingResponse {
    let owner_handle = repo.record.owner_handle.clone();
    let repo_name = repo.record.name.clone();
    match landing {
        RepositoryInviteLanding::Open(viewer_state) => RepositoryInviteLandingResponse::Open {
            viewer: match viewer_state {
                RepositoryInviteViewer::Ready => RepositoryInviteViewerResponse::Ready,
                RepositoryInviteViewer::SignedOut => RepositoryInviteViewerResponse::SignedOut,
                RepositoryInviteViewer::WrongAccount => {
                    RepositoryInviteViewerResponse::WrongAccount
                }
                RepositoryInviteViewer::EmailUnverified => {
                    RepositoryInviteViewerResponse::EmailUnverified
                }
            },
            viewer_email: viewer.map(|viewer| viewer.email.clone()),
            owner_handle,
            repo_name,
            invited_email: invite.invited_email.clone(),
            permissions: invite.permissions.into(),
            expires_at_unix: invite.expires_at_unix,
        },
        RepositoryInviteLanding::Member => RepositoryInviteLandingResponse::Member {
            owner_handle,
            repo_name,
        },
        RepositoryInviteLanding::Expired => RepositoryInviteLandingResponse::Expired {
            owner_handle,
            repo_name,
        },
        RepositoryInviteLanding::Revoked => RepositoryInviteLandingResponse::Revoked,
        RepositoryInviteLanding::AccessRemoved => RepositoryInviteLandingResponse::AccessRemoved,
        RepositoryInviteLanding::Used => RepositoryInviteLandingResponse::Used,
    }
}
