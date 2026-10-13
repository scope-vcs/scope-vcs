use scope_domain::repository::{
    RepoLifecycleState,
    access::{RepositoryAccess, repository_access_for_user_id},
    collaboration::RepositoryMemberPermissions,
};

pub(crate) const OWNER: &str = "user_owner";

pub(crate) fn owner() -> RepositoryAccess {
    repository_access_for_user_id(OWNER, RepoLifecycleState::Ready, None, OWNER)
}

pub(crate) fn member() -> RepositoryAccess {
    repository_access_for_user_id(
        OWNER,
        RepoLifecycleState::Ready,
        Some(RepositoryMemberPermissions::default()),
        "user_member",
    )
}

pub(crate) fn outsider() -> RepositoryAccess {
    repository_access_for_user_id(OWNER, RepoLifecycleState::Ready, None, "user_outsider")
}
