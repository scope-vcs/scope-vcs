use super::{
    request_queue::{SEARCH_MAX_CHARS, requested_request_views},
    requests::repo_metadata_and_access,
    responses::recorded_actor_response,
};
use crate::{auth::viewer::Viewer, error::ApiError, state::AppState};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use scope_api_contract::{
    RequestBacklogItemResponse, RequestBacklogPageResponse, RequestBacklogQuery,
};
use scope_domain::requests::{
    REQUEST_LIST_DEFAULT_PAGE_SIZE, REQUEST_LIST_MAX_PAGE_SIZE, RequestBacklogSearch,
    RequestViewer, backlog_request_closable,
};
use scope_postgres::db::{RequestBacklogCursor, RequestBacklogPageQuery};

pub(crate) async fn request_backlog(
    State(state): State<AppState>,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<RequestBacklogQuery>,
    viewer: Viewer,
) -> Result<Json<RequestBacklogPageResponse>, ApiError> {
    let (repo, viewer_user_id) =
        repo_metadata_and_access(&state, &viewer, &owner, &repo_name).await?;
    let request_views = requested_request_views(&repo, query.view)?;
    let after = query.cursor.as_deref().map(parse_cursor).transpose()?;
    let search = query
        .search
        .as_deref()
        .and_then(RequestBacklogSearch::parse);
    if query
        .search
        .is_some_and(|search| search.trim().chars().count() > SEARCH_MAX_CHARS)
    {
        return Err(ApiError::bad_request("request backlog search is too long"));
    }
    let limit = query
        .limit
        .unwrap_or(REQUEST_LIST_DEFAULT_PAGE_SIZE)
        .clamp(1, REQUEST_LIST_MAX_PAGE_SIZE);
    let viewer = RequestViewer::new(repo.access.clone(), viewer_user_id.as_deref(), false);
    let mut page = state
        .metadata
        .requests()
        .request_backlog_page(RequestBacklogPageQuery {
            repo_id: &repo.record.id,
            viewer: &viewer,
            views: &repo.views,
            request_views,
            search: search.as_ref(),
            working: query.working.map(Into::into).unwrap_or_default(),
            sort: query.sort.map(Into::into).unwrap_or_default(),
            after: after.as_ref(),
            limit: (limit + 1) as u64,
        })
        .await?;
    let has_more = page.rows.len() > limit;
    page.rows.truncate(limit);
    let next_cursor = page
        .rows
        .last()
        .filter(|_| has_more)
        .map(|row| encode_cursor(&row.cursor()));
    let requests = page
        .rows
        .into_iter()
        .map(|row| {
            Ok(RequestBacklogItemResponse {
                can_close: backlog_request_closable(row.worker_user_id.as_deref(), &viewer),
                worker: recorded_actor_response(row.worker_user_id.as_deref(), &page.users)?,
                id: row.request_id,
                title: row.title,
                github_issue_number: row.github_issue_number,
                asker_login: row.asker_login,
                asked_at_unix: row.asked_at_unix,
                message_count: row.message_count,
            })
        })
        .collect::<Result<_, ApiError>>()?;
    Ok(Json(RequestBacklogPageResponse {
        requests,
        next_cursor,
        total: page.total,
    }))
}

fn encode_cursor(cursor: &RequestBacklogCursor) -> String {
    format!("{}.{}", cursor.asked_at_unix, cursor.github_issue_number)
}

fn parse_cursor(value: &str) -> Result<RequestBacklogCursor, ApiError> {
    let invalid = || ApiError::bad_request("invalid request backlog cursor");
    let (asked_at_unix, github_issue_number) = value.split_once('.').ok_or_else(invalid)?;
    let position = |digits: &str| {
        digits
            .parse::<u64>()
            .ok()
            .filter(|position| i64::try_from(*position).is_ok())
            .ok_or_else(invalid)
    };
    Ok(RequestBacklogCursor {
        asked_at_unix: position(asked_at_unix)?,
        github_issue_number: position(github_issue_number)?,
    })
}
