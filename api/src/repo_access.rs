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

pub(crate) async fn require_full_view_member(
    state: &AppState,
    user_id: &str,
    owner: &str,
    name: &str,
) -> Result<scope_domain::repository::access::RepositoryAccessContext, ApiError> {
    let repo = state
        .metadata
        .repositories()
        .repository_access(owner, name, Some(user_id))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
    if !repo.access.is_maintainer() {
        find_read_access(state, owner, name, Some(user_id)).await?;
    }
    repo.ensure_member()?;
    repo.ensure_run_reader()?;
    Ok(repo)
}
