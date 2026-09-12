use super::{RepoLifecycleState, RepoRecord, Repository, RepositoryIncarnation};
use crate::{
    policy::{Principal, PrincipalKind, ScopePath},
    repository::collaboration::RepositoryMemberPermissions,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryActor {
    Public,
    Member,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryAccess {
    pub actor: RepositoryActor,
    pub can_read_private_files: bool,
    pub can_push: bool,
    pub can_change_file_visibility: bool,
    pub can_apply_changes: bool,
    pub can_manage_members: bool,
    pub can_delete_repo: bool,
}

/// The repository identity and permissions required for one viewer's metadata operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryAccessContext {
    pub record: RepoRecord,
    pub access: RepositoryAccess,
    pub root_visibility: crate::policy::Visibility,
}

impl RepositoryAccessContext {
    pub fn can_read_root(&self) -> bool {
        self.access.can_read_private_files
            || self.root_visibility == crate::policy::Visibility::Public
    }
    pub fn incarnation(&self) -> RepositoryIncarnation {
        self.record.incarnation()
    }

    pub fn ensure_member(&self) -> Result<(), crate::error::DomainError> {
        if self.access.is_maintainer() {
            Ok(())
        } else {
            Err(crate::error::DomainError::forbidden(
                "repo membership required",
            ))
        }
    }

    pub fn can_read(&self, public_files_visible: bool) -> bool {
        can_read_repository(
            self.record.lifecycle_state,
            self.access,
            public_files_visible,
        )
    }
}

/// Whether a viewer with `access` may read the repository at all. Public
/// viewers additionally need the public projection to expose at least one file.
pub fn can_read_repository(
    lifecycle_state: RepoLifecycleState,
    access: RepositoryAccess,
    public_files_visible: bool,
) -> bool {
    match access.actor {
        RepositoryActor::Owner => true,
        RepositoryActor::Member => lifecycle_state == RepoLifecycleState::Ready,
        RepositoryActor::Public => {
            lifecycle_state == RepoLifecycleState::Ready && public_files_visible
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainPushMode {
    Denied,
    FirstPush,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepositoryPushPolicy {
    pub access: RepositoryAccess,
    pub mode: MainPushMode,
}

impl RepositoryAccess {
    pub fn main_push_mode(self, lifecycle_state: RepoLifecycleState) -> MainPushMode {
        if lifecycle_state == RepoLifecycleState::AwaitingFirstPush
            && self.actor == RepositoryActor::Owner
        {
            MainPushMode::FirstPush
        } else if lifecycle_state == RepoLifecycleState::Ready && self.can_push {
            MainPushMode::Ready
        } else {
            MainPushMode::Denied
        }
    }

    pub fn is_maintainer(self) -> bool {
        matches!(self.actor, RepositoryActor::Owner | RepositoryActor::Member)
    }

    /// The repository change counter a viewer may see. Private-only mutations
    /// advance it too, so public viewers get 0 rather than a signal of
    /// activity they cannot read.
    pub fn visible_change_version(self, change_version: u64) -> u64 {
        if self.actor == RepositoryActor::Public {
            0
        } else {
            change_version
        }
    }

    pub fn public() -> Self {
        Self {
            actor: RepositoryActor::Public,
            can_read_private_files: false,
            can_push: false,
            can_change_file_visibility: false,
            can_apply_changes: false,
            can_manage_members: false,
            can_delete_repo: false,
        }
    }
}

pub fn repository_access_for_user_id(
    owner_user_id: &str,
    lifecycle_state: RepoLifecycleState,
    member_permissions: Option<RepositoryMemberPermissions>,
    user_id: &str,
) -> RepositoryAccess {
    let ready = lifecycle_state == RepoLifecycleState::Ready;
    if owner_user_id == user_id {
        return RepositoryAccess {
            actor: RepositoryActor::Owner,
            can_read_private_files: true,
            can_push: ready,
            can_change_file_visibility: true,
            can_apply_changes: true,
            can_manage_members: ready,
            can_delete_repo: true,
        };
    }

    let Some(permissions) = member_permissions else {
        return RepositoryAccess::public();
    };
    RepositoryAccess {
        actor: RepositoryActor::Member,
        can_read_private_files: ready,
        can_push: ready && permissions.can_push,
        can_change_file_visibility: ready && permissions.can_change_file_visibility,
        can_apply_changes: ready && permissions.can_apply_changes,
        can_manage_members: false,
        can_delete_repo: false,
    }
}

pub fn repository_push_policy_for_user_id(
    owner_user_id: &str,
    lifecycle_state: RepoLifecycleState,
    member_permissions: Option<RepositoryMemberPermissions>,
    user_id: &str,
) -> RepositoryPushPolicy {
    let access =
        repository_access_for_user_id(owner_user_id, lifecycle_state, member_permissions, user_id);
    let mode = access.main_push_mode(lifecycle_state);
    RepositoryPushPolicy { access, mode }
}

impl Repository {
    pub fn access_for_principal(&self, principal: &Principal) -> RepositoryAccess {
        if principal.kind == PrincipalKind::Public {
            return RepositoryAccess::public();
        }

        self.access_for_user_id(&principal.id)
    }

    pub fn access_for_user_id(&self, user_id: &str) -> RepositoryAccess {
        repository_access_for_user_id(
            &self.record.owner_user_id,
            self.record.lifecycle_state,
            self.member_for_user(user_id)
                .map(|member| member.permissions),
            user_id,
        )
    }

    pub fn can_read_path(&self, principal: &Principal, path: &ScopePath) -> bool {
        if principal.kind == PrincipalKind::Public {
            return self.record.lifecycle_state == RepoLifecycleState::Ready
                && self.policy.can_read(path, false);
        }

        let access = self.access_for_principal(principal);
        match access.actor {
            RepositoryActor::Owner => self.policy.can_read(path, true),
            RepositoryActor::Member => {
                self.record.lifecycle_state == RepoLifecycleState::Ready
                    && self.policy.can_read(path, access.can_read_private_files)
            }
            RepositoryActor::Public => false,
        }
    }

    pub fn can_push(&self, principal: &Principal) -> bool {
        self.access_for_principal(principal).can_push
    }

    pub fn push_policy_for_user_id(&self, user_id: &str) -> RepositoryPushPolicy {
        repository_push_policy_for_user_id(
            &self.record.owner_user_id,
            self.record.lifecycle_state,
            self.member_for_user(user_id)
                .map(|member| member.permissions),
            user_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_viewers_never_see_the_change_version() {
        assert_eq!(RepositoryAccess::public().visible_change_version(7), 0);
        let owner =
            repository_access_for_user_id("owner", RepoLifecycleState::Ready, None, "owner");
        assert_eq!(owner.visible_change_version(7), 7);
        let member = repository_access_for_user_id(
            "owner",
            RepoLifecycleState::Ready,
            Some(RepositoryMemberPermissions::default()),
            "member",
        );
        assert_eq!(member.visible_change_version(7), 7);
    }

    #[test]
    fn repository_readability_follows_actor_and_lifecycle() {
        let owner =
            repository_access_for_user_id("owner", RepoLifecycleState::Ready, None, "owner");
        let member = repository_access_for_user_id(
            "owner",
            RepoLifecycleState::Ready,
            Some(RepositoryMemberPermissions::default()),
            "member",
        );
        for state in [
            RepoLifecycleState::AwaitingFirstPush,
            RepoLifecycleState::Ready,
        ] {
            assert!(can_read_repository(state, owner, false));
        }
        assert!(can_read_repository(
            RepoLifecycleState::Ready,
            member,
            false
        ));
        assert!(!can_read_repository(
            RepoLifecycleState::AwaitingFirstPush,
            member,
            true
        ));
        let public = RepositoryAccess::public();
        assert!(can_read_repository(RepoLifecycleState::Ready, public, true));
        assert!(!can_read_repository(
            RepoLifecycleState::Ready,
            public,
            false
        ));
        assert!(!can_read_repository(
            RepoLifecycleState::AwaitingFirstPush,
            public,
            true
        ));
    }

    #[test]
    fn main_push_policy_keeps_first_push_owner_only_and_honors_member_permissions() {
        for (state, user, permissions, expected) in [
            (
                RepoLifecycleState::AwaitingFirstPush,
                "owner",
                None,
                MainPushMode::FirstPush,
            ),
            (
                RepoLifecycleState::Ready,
                "owner",
                None,
                MainPushMode::Ready,
            ),
            (
                RepoLifecycleState::AwaitingFirstPush,
                "visitor",
                None,
                MainPushMode::Denied,
            ),
            (
                RepoLifecycleState::Ready,
                "visitor",
                None,
                MainPushMode::Denied,
            ),
            (
                RepoLifecycleState::AwaitingFirstPush,
                "member",
                Some(true),
                MainPushMode::Denied,
            ),
            (
                RepoLifecycleState::Ready,
                "member",
                Some(true),
                MainPushMode::Ready,
            ),
            (
                RepoLifecycleState::Ready,
                "member",
                Some(false),
                MainPushMode::Denied,
            ),
        ] {
            let permissions = permissions.map(|can_push| RepositoryMemberPermissions {
                can_push,
                can_change_file_visibility: false,
                can_apply_changes: false,
            });
            let policy = repository_push_policy_for_user_id("owner", state, permissions, user);
            assert_eq!(policy.mode, expected, "{state:?} {user}");
            assert_eq!(policy.access.main_push_mode(state), expected);
        }
    }
}
