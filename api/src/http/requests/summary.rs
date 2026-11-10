use super::request_invitee_response;
use crate::{
    error::ApiError,
    http::responses::{RequestGateOutcomes, RequestMains, request_response},
    state::AppState,
};
use scope_api_contract::RequestSummaryResponse;
use scope_domain::{
    requests::{Request, RequestViewer, request_policy},
    views::Views,
};

pub(super) async fn request_response_for_viewer(
    state: &AppState,
    request: Request,
    viewer: RequestViewer<'_>,
    views: &Views,
    current_main_oid: Option<String>,
    repository_lifecycle: scope_domain::repository::RepoLifecycleState,
) -> Result<RequestSummaryResponse, ApiError> {
    let policy = request_policy(&request, viewer.clone(), views);
    let invitees = if views.anyone() == Some(&request.view) && policy.exact_visible {
        state
            .metadata
            .requests()
            .request_invitees(&request.id)
            .await?
            .into_iter()
            .map(request_invitee_response)
            .collect()
    } else {
        Vec::new()
    };
    let checks = crate::use_cases::request_checks::recorded_checks_view(state, &request)
        .await?
        .outcome;
    let reviews = state
        .metadata
        .requests()
        .request_reviews_outcome(&request, views)
        .await?;
    let view_mains = state
        .metadata
        .requests()
        .request_view_mains(&request.repo_id)
        .await?;
    request_response(
        request,
        viewer,
        views,
        RequestMains {
            viewer_main_oid: current_main_oid.as_deref(),
            view_mains: &view_mains,
        },
        invitees,
        RequestGateOutcomes { checks, reviews },
        repository_lifecycle,
    )
}
