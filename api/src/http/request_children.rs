use crate::{error::ApiError, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_api_contract::RequestChildrenPageResponse;
use scope_domain::requests::REQUEST_LIST_DEFAULT_PAGE_SIZE;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct ChildrenQuery {
    cursor: Option<String>,
    limit: Option<usize>,
}

pub(crate) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Query(query): Query<ChildrenQuery>,
) -> Result<Json<RequestChildrenPageResponse>, ApiError> {
    let after_id = query
        .cursor
        .as_deref()
        .map(super::requests::parse_request_list_cursor)
        .transpose()?;
    let user = crate::auth::scope::optional_scope_user(&state, &headers).await?;
    let snapshot = state
        .metadata
        .requests()
        .request_state_snapshot_with_children(
            &owner,
            &repo_name,
            &request_id,
            user.as_ref().map(|user| user.id.as_str()),
            after_id.as_deref(),
            query.limit.unwrap_or(REQUEST_LIST_DEFAULT_PAGE_SIZE),
        )
        .await?;
    Ok(Json(super::responses::request_children_page_response(
        &snapshot.children,
    )?))
}
