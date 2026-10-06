use crate::{error::ApiError, repo_access::ensure_repo_read};
use scope_domain::{
    policy::Principal, repository::Repository, repository::access::RepositoryActor, views::ViewId,
};

pub(crate) fn ensure_projection_preview_access(
    repo: &Repository,
    requester: &Principal,
    view: &ViewId,
) -> Result<(), ApiError> {
    let access = repo.access_for_principal(requester);
    if !repo.can_read_view(&access, view) {
        return Err(ApiError::forbidden("view access required"));
    }
    if access.actor == RepositoryActor::Public {
        ensure_repo_read(repo, &Principal::public())
    } else {
        ensure_repo_read(repo, requester)
    }
}
