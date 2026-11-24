use super::requests::{actor_request, referenced_request_ids};
use crate::{
    after_commit::AfterCommit,
    auth::viewer::Viewer,
    error::ApiError,
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::{github::review_bot_statuses::reviews_outcomes, request_checks::checks_outcomes},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{
    EditRequestLinksRequest, LinkedRequestResponse, RequestDetailResponse, RequestLinksResponse,
};
use scope_domain::requests::{
    RequestChecksOutcome, RequestReviewsOutcome, RequestViewer, VisibleLinkedRequest,
    request_mergeability,
};
use scope_postgres::db::RequestStateSnapshot;

pub(crate) async fn edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Json(input): Json<EditRequestLinksRequest>,
) -> Result<Json<RequestDetailResponse>, ApiError> {
    let (user, repo, request) =
        actor_request(&state, &headers, &owner, &repo_name, &request_id).await?;
    state
        .metadata
        .requests()
        .edit_request_links(
            &request.id,
            &user.id,
            &referenced_request_ids(&state, &repo.record.id, &input.resolve).await?,
            &referenced_request_ids(&state, &repo.record.id, &input.stop_resolving).await?,
            unix_now()?,
        )
        .await?;
    state.after_commit([AfterCommit::request_summary(
        &repo.incarnation(),
        RepoChangeReason::RequestLinksChanged,
    )]);
    let snapshot = super::request_state::load(
        &state,
        &Viewer::resolve(&state, &headers).await?,
        &owner,
        &repo_name,
        &request.id,
    )
    .await?;
    super::request_state::full_detail_response(&state, &snapshot)
        .await
        .map(Json)
}

pub(crate) async fn response(
    state: &AppState,
    snapshot: &RequestStateSnapshot,
) -> Result<RequestLinksResponse, ApiError> {
    let links = &snapshot.links;
    let heads = links
        .shown()
        .filter_map(VisibleLinkedRequest::readable_change)
        .map(Into::into)
        .collect::<Vec<_>>();
    let repo_id = &snapshot.repository.record.id;
    let views = &snapshot.repository.views;
    let (checks, reviews) = if heads.is_empty() {
        Default::default()
    } else {
        (
            checks_outcomes(state, repo_id, &heads).await?,
            reviews_outcomes(state, repo_id, views, &heads).await?,
        )
    };
    let linked = |linked: &VisibleLinkedRequest| {
        let request = &linked.request;
        let viewer = RequestViewer::new(
            snapshot.repository.access.clone(),
            snapshot.viewer_user_id.as_deref(),
            linked.viewer_is_invitee,
        );
        Ok::<_, ApiError>(LinkedRequestResponse {
            id: request.id.clone(),
            name: request.name.clone(),
            title: request.title.clone(),
            author: super::responses::recorded_actor_response(
                request.author_user_id.as_deref(),
                &snapshot.users,
            )?,
            status: request_mergeability(
                request,
                &snapshot.view_mains,
                &viewer,
                views,
                checks
                    .get(&request.id)
                    .copied()
                    .unwrap_or(RequestChecksOutcome::NotEvaluated),
                reviews
                    .get(&request.id)
                    .copied()
                    .unwrap_or(RequestReviewsOutcome::Clear),
                snapshot.repository.record.lifecycle_state,
            )
            .status
            .into(),
        })
    };
    Ok(RequestLinksResponse {
        resolves: links
            .resolves
            .iter()
            .map(linked)
            .collect::<Result<_, _>>()?,
        resolved_by: links
            .resolved_by
            .iter()
            .map(linked)
            .collect::<Result<_, _>>()?,
        close_named: links.close_named.as_ref().map(linked).transpose()?,
    })
}
