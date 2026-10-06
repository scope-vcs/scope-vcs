use super::{RepoLifecycleState, RepoRecord, Repository, RepositoryIncarnation};
use crate::{
    policy::{Principal, PrincipalKind, ScopePath},
    repository::collaboration::RepositoryMemberPermissions,
    views::{ViewId, Views},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryActor {
    Public,
    Member,
    Owner,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryAccess {
    pub actor: RepositoryActor,
    pub view: ViewId,
    pub can_push: bool,
    pub can_change_file_visibility: bool,
    pub can_manage_members: bool,
    pub can_delete_repo: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryAccessContext {
    pub record: RepoRecord,
    pub access: RepositoryAccess,
}

impl RepositoryAccessContext {
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

    pub fn ensure_owner(&self) -> Result<(), crate::error::DomainError> {
        if self.access.actor == RepositoryActor::Owner {
            Ok(())
        } else {
            Err(crate::error::DomainError::forbidden("owner role required"))
        }
    }

    pub fn can_read(&self, public_files_visible: bool) -> bool {
        can_read_repository(
            self.record.lifecycle_state,
            &self.access,
            public_files_visible,
        )
    }
}

pub fn can_read_repository(
    lifecycle_state: RepoLifecycleState,
    access: &RepositoryAccess,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryPushPolicy {
    pub access: RepositoryAccess,
    pub mode: MainPushMode,
}

impl RepositoryAccess {
    pub fn main_push_mode(&self, lifecycle_state: RepoLifecycleState) -> MainPushMode {
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

    pub fn is_maintainer(&self) -> bool {
        matches!(self.actor, RepositoryActor::Owner | RepositoryActor::Member)
    }

    pub fn visible_version(&self, version: u64) -> u64 {
        if self.actor == RepositoryActor::Public {
            0
        } else {
            version
        }
    }

    pub fn public() -> Self {
        Self {
            actor: RepositoryActor::Public,
            view: ViewId::public(),
            can_push: false,
            can_change_file_visibility: false,
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
            view: Views::builtin().full().clone(),
            can_push: ready,
            can_change_file_visibility: true,
            can_manage_members: ready,
            can_delete_repo: true,
        };
    }

    let Some(permissions) = member_permissions else {
        return RepositoryAccess::public();
    };
    RepositoryAccess {
        actor: RepositoryActor::Member,
        view: permissions.view,
        can_push: ready && permissions.can_push,
        can_change_file_visibility: ready && permissions.can_change_file_visibility,
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
                .map(|member| member.permissions.clone()),
            user_id,
        )
    }

    pub fn can_read_path(&self, principal: &Principal, path: &ScopePath) -> bool {
        if principal.kind == PrincipalKind::Public {
            return self.record.lifecycle_state == RepoLifecycleState::Ready
                && self.repo_config.views.anyone().is_some_and(|view| {
                    self.policy.can_read(path, view, self.repo_config.views())
                });
        }

        let access = self.access_for_principal(principal);
        match access.actor {
            RepositoryActor::Owner => {
                self.policy
                    .can_read(path, &access.view, self.repo_config.views())
            }
            RepositoryActor::Member => {
                self.record.lifecycle_state == RepoLifecycleState::Ready
                    && self
                        .policy
                        .can_read(path, &access.view, self.repo_config.views())
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
                .map(|member| member.permissions.clone()),
            user_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_viewers_never_see_the_change_version() {
        assert_eq!(RepositoryAccess::public().visible_version(7), 0);
        let owner =
            repository_access_for_user_id("owner", RepoLifecycleState::Ready, None, "owner");
        assert_eq!(owner.visible_version(7), 7);
        assert_eq!(owner.view, Views::builtin().full().clone());
        let member = repository_access_for_user_id(
            "owner",
            RepoLifecycleState::Ready,
            Some(RepositoryMemberPermissions::default()),
            "member",
        );
        assert_eq!(member.visible_version(7), 7);
        assert_eq!(member.view, ViewId::private());
        let public_member = repository_access_for_user_id(
            "owner",
            RepoLifecycleState::Ready,
            Some(RepositoryMemberPermissions {
                view: ViewId::public(),
                ..RepositoryMemberPermissions::default()
            }),
            "member",
        );
        assert_eq!(public_member.view, ViewId::public());
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
            assert!(can_read_repository(state, &owner, false));
        }
        assert!(can_read_repository(
            RepoLifecycleState::Ready,
            &member,
            false
        ));
        assert!(!can_read_repository(
            RepoLifecycleState::AwaitingFirstPush,
            &member,
            true
        ));
        let public = RepositoryAccess::public();
        assert!(can_read_repository(
            RepoLifecycleState::Ready,
            &public,
            true
        ));
        assert!(!can_read_repository(
            RepoLifecycleState::Ready,
            &public,
            false
        ));
        assert!(!can_read_repository(
            RepoLifecycleState::AwaitingFirstPush,
            &public,
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
                view: ViewId::private(),
            });
            let policy = repository_push_policy_for_user_id("owner", state, permissions, user);
            assert_eq!(policy.mode, expected, "{state:?} {user}");
            assert_eq!(policy.access.main_push_mode(state), expected);
        }
    }
}
