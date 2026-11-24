use crate::{auth::viewer::Viewer, error::ApiError, persistence::unix_now, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
};
use scope_api_contract::{RequestDetailResponse, RequestLinksResponse, RequestStateResponse};
use scope_domain::requests::{DiffStat, RequestViewer, request_policy};
use scope_postgres::db::RequestStateSnapshot;

pub(crate) async fn get(
    State(state): State<AppState>,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    viewer: Viewer,
) -> Result<Json<RequestStateResponse>, ApiError> {
    let snapshot = load(&state, &viewer, &owner, &repo_name, &request_id).await?;
    let viewer = self::viewer(&snapshot);
    Ok(Json(RequestStateResponse {
        viewer: snapshot
            .viewer_user_id
            .as_deref()
            .map(|id| super::responses::request_actor_summary_response(id, &snapshot.users))
            .transpose()?,
        detail: full_detail_response(&state, &snapshot).await?,
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
    viewer: &Viewer,
    owner: &str,
    repo_name: &str,
    request_id: &str,
) -> Result<RequestStateSnapshot, ApiError> {
    state
        .metadata
        .requests()
        .request_state_snapshot(owner, repo_name, request_id, viewer.user_id())
        .await
        .map_err(Into::into)
}

pub(crate) async fn diff_stat(
    state: &AppState,
    snapshot: &RequestStateSnapshot,
) -> Result<Option<DiffStat>, ApiError> {
    let policy = request_policy(
        &snapshot.request,
        viewer(snapshot),
        &snapshot.repository.views,
    );
    if !policy.change_visible {
        return Ok(None);
    }
    Ok(
        crate::use_cases::request_diff_stats::readable_request_diff_stats(
            state,
            &snapshot.repository,
            std::slice::from_ref(&snapshot.request.id),
        )
        .await?
        .remove(&snapshot.request.id),
    )
}

pub(crate) async fn full_detail_response(
    state: &AppState,
    snapshot: &RequestStateSnapshot,
) -> Result<RequestDetailResponse, ApiError> {
    detail_response(
        snapshot,
        viewer(snapshot),
        diff_stat(state, snapshot).await?,
        super::request_links::response(state, snapshot).await?,
    )
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
    diff_stat: Option<DiffStat>,
    links: RequestLinksResponse,
) -> Result<RequestDetailResponse, ApiError> {
    let checks = snapshot.checks_outcome();
    Ok(RequestDetailResponse {
        author: super::responses::recorded_actor_response(
            snapshot.request.author_user_id.as_deref(),
            &snapshot.users,
        )?,
        author_record: snapshot
            .author_record
            .map(super::responses::author_record_response),
        claimer: super::responses::recorded_actor_response(
            snapshot.claimer_user_id.as_deref(),
            &snapshot.users,
        )?,
        asker: snapshot
            .asker
            .as_ref()
            .map(super::responses::github_account_response),
        diff_stat: diff_stat.map(super::responses::diff_stat_response),
        links,
        stack_navigation: scope_api_contract::RequestStackNavigationResponse {
            parent: snapshot
                .stack_navigation
                .parent
                .clone()
                .map(super::responses::request_stack_parent_response)
                .transpose()?,
            ancestors: snapshot
                .stack_navigation
                .ancestors
                .iter()
                .cloned()
                .map(super::responses::request_stack_node_response)
                .collect::<Result<_, _>>()?,
            ancestors_unavailable: snapshot.stack_navigation.ancestors_unavailable,
            children: super::responses::request_children_page_response(&snapshot.children)?,
        },
        request: super::responses::request_response(
            snapshot.request.clone(),
            viewer,
            &snapshot.repository.views,
            super::responses::RequestMains {
                viewer_main_oid: snapshot.current_main_oid.as_deref(),
                view_mains: &snapshot.view_mains,
            },
            snapshot
                .invitees
                .iter()
                .cloned()
                .map(super::requests::request_invitee_response)
                .collect(),
            super::responses::RequestGateOutcomes {
                checks,
                reviews: snapshot.reviews_outcome(),
            },
            snapshot.repository.record.lifecycle_state,
        )?,
    })
}
