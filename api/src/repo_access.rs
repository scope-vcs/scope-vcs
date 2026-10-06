use crate::{error::ApiError, state::AppState};

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
