use crate::{
    error::ApiError, http::responses::ProjectionPreviewAudience, repo_access::ensure_repo_read,
};
use scope_domain::{
    policy::Principal, repository::Repository, repository::access::RepositoryActor,
};

pub(crate) fn ensure_projection_preview_access(
    repo: &Repository,
    requester: &Principal,
    audience: ProjectionPreviewAudience,
) -> Result<(), ApiError> {
    match audience {
        ProjectionPreviewAudience::Private => {
            ensure_repo_read(repo, requester)?;
            if repo.access_for_principal(requester).actor != RepositoryActor::Public {
                Ok(())
            } else {
                Err(ApiError::forbidden("repo membership required"))
            }
        }
        ProjectionPreviewAudience::Public => {
            if repo.access_for_principal(requester).actor != RepositoryActor::Public {
                ensure_repo_read(repo, requester)
            } else {
                ensure_repo_read(repo, &Principal::public())
            }
        }
    }
}
