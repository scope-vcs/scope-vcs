use crate::{error::ApiError, state::AppState};
use scope_domain::{
    repository::access::RepositoryAccess,
    requests::{Request, RequestPolicyDecision, RequestViewer, request_policy},
};

pub(crate) async fn request_policy_for_viewer(
    state: &AppState,
    request: &Request,
    access: RepositoryAccess,
    viewer_user_id: Option<&str>,
) -> Result<RequestPolicyDecision, ApiError> {
    let is_invitee = match viewer_user_id {
        Some(user_id) => {
            state
                .metadata
                .requests()
                .request_is_invitee(&request.id, user_id)
                .await?
        }
        None => false,
    };
    Ok(request_policy(
        request,
        RequestViewer::new(access, viewer_user_id, is_invitee),
    ))
}

pub(crate) async fn visible_request(
    state: &AppState,
    repo_id: &str,
    access: RepositoryAccess,
    viewer_user_id: Option<&str>,
    request_id: &str,
) -> Result<Request, ApiError> {
    let request = state
        .metadata
        .requests()
        .request_by_id(request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    let policy = request_policy_for_viewer(state, &request, access, viewer_user_id).await?;
    if request.repo_id != repo_id || !policy.exact_visible {
        return Err(ApiError::not_found("request not found"));
    }
    Ok(request)
}
