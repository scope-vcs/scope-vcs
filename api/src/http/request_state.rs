use crate::{
    auth::scope::optional_scope_user, error::ApiError, persistence::unix_now, state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{RequestDetailResponse, RequestStateResponse};
use scope_domain::requests::RequestViewer;
use scope_postgres::db::RequestStateSnapshot;

pub(crate) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
) -> Result<Json<RequestStateResponse>, ApiError> {
    let snapshot = load(&state, &headers, &owner, &repo_name, &request_id).await?;
    let viewer = viewer(&snapshot);
    Ok(Json(RequestStateResponse {
        viewer: snapshot
            .viewer_user_id
            .as_deref()
            .map(|id| super::responses::request_actor_summary_response(id, &snapshot.users))
            .transpose()?,
        detail: detail_response(&snapshot, viewer.clone())?,
        checks: super::request_checks::snapshot_response(
            &state,
            &snapshot,
            viewer.clone(),
            unix_now()?,
        )?,
        auto_merge: super::request_auto_merge::snapshot_response(&snapshot, viewer)?,
    }))
}

pub(crate) async fn load(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    request_id: &str,
) -> Result<RequestStateSnapshot, ApiError> {
    let user = optional_scope_user(state, headers).await?;
    state
        .metadata
        .requests()
        .request_state_snapshot(
            owner,
            repo_name,
            request_id,
            user.as_ref().map(|user| user.id.as_str()),
        )
        .await
        .map_err(Into::into)
}

pub(crate) fn viewer(snapshot: &RequestStateSnapshot) -> RequestViewer<'_> {
    RequestViewer::new(
        snapshot.repository.access.clone(),
        snapshot.viewer_user_id.as_deref(),
        snapshot.is_invitee,
    )
}

pub(crate) fn detail_response(
    snapshot: &RequestStateSnapshot,
    viewer: RequestViewer<'_>,
) -> Result<RequestDetailResponse, ApiError> {
    let checks = scope_domain::requests::request_checks_outcome(
        &snapshot.request.id,
        &snapshot.request.head_oid,
        snapshot.evaluation.as_ref(),
        &snapshot.results,
    );
    Ok(RequestDetailResponse {
        request: super::requests::request_response(
            snapshot.request.clone(),
            viewer,
            &snapshot.repository.views,
            snapshot.current_main_oid.clone(),
            snapshot
                .invitees
                .iter()
                .cloned()
                .map(super::requests::request_invitee_response)
                .collect(),
            checks,
        )?,
    })
}
