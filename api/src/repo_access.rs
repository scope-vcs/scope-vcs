use crate::{error::ApiError, state::AppState};
use scope_domain::{
    policy::{Principal, ScopePath},
    projection_views::has_visible_projected_non_control_files,
    repository::access::RepositoryActor,
    repository::{RepoLifecycleState, Repository},
};

pub(crate) async fn find_read_access(
    state: &AppState,
    owner: &str,
    name: &str,
    viewer_user_id: Option<&str>,
) -> Result<scope_domain::repository::access::RepositoryAccessContext, ApiError> {
    state
        .metadata
        .repositories()
        .repository_read_access(owner, name, viewer_user_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))
}

pub(crate) async fn find_repo(
    state: &AppState,
    owner: &str,
    name: &str,
) -> Result<Repository, ApiError> {
    state
        .metadata
        .repositories()
        .repository(owner, name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))
}

pub(crate) fn ensure_repo_read(
    _state: &AppState,
    repo: &Repository,
    principal: &Principal,
) -> Result<(), ApiError> {
    let access = repo.access_for_principal(principal);
    let readable = if access.actor == RepositoryActor::Public {
        repo.record.lifecycle_state == RepoLifecycleState::Ready
            && has_visible_projected_non_control_files(repo, principal)
    } else {
        repo.can_read_path(principal, &ScopePath::root())
    };

    if readable {
        Ok(())
    } else {
        Err(ApiError::not_found(format!(
            "repo {} not found",
            repo.record.id
        )))
    }
}
