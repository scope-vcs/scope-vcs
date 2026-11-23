use super::FakeGitHub;
use axum::{
    Json, Router,
    extract::{Path as AxumPath, State as AxumState},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::sync::{Arc, atomic::Ordering};

impl FakeGitHub {
    pub(in crate::workflow_tests) fn report_review_thread(
        &self,
        root_comment_id: u64,
        resolved_by: Option<(&str, u64)>,
    ) {
        let node_id = format!("PRRT_{root_comment_id}");
        self.review_threads.lock().unwrap().insert(
            node_id.clone(),
            serde_json::json!({
                "id": node_id,
                "isResolved": resolved_by.is_some(),
                "resolvedBy": resolved_by.map(|(login, id)| serde_json::json!({
                    "login": login, "databaseId": id, "avatarUrl": null,
                })),
                "comments": { "nodes": [{ "fullDatabaseId": root_comment_id.to_string() }] },
            }),
        );
    }

    pub(in crate::workflow_tests) fn report_permission(
        &self,
        login: &str,
        permission: Option<(&'static str, u64)>,
    ) {
        self.collaborator_permissions
            .lock()
            .unwrap()
            .insert(login.to_string(), permission);
    }

    pub(in crate::workflow_tests) fn report_pull_review(
        &self,
        review: serde_json::Value,
        comments: Vec<serde_json::Value>,
    ) {
        let mut reported = self.review_comments.lock().unwrap();
        for mut comment in comments {
            comment["pull_request_review_id"] = review["id"].clone();
            reported.insert(comment["id"].as_u64().unwrap(), comment);
        }
        self.pull_reviews
            .lock()
            .unwrap()
            .insert(review["id"].as_u64().unwrap(), review);
    }
}

fn review_listing(comment: &serde_json::Value) -> serde_json::Value {
    let mut listed = comment.clone();
    for field in [
        "subject_type",
        "side",
        "line",
        "start_line",
        "original_line",
        "original_start_line",
    ] {
        listed[field] = serde_json::Value::Null;
    }
    listed
}

fn found(value: Option<serde_json::Value>) -> Response {
    match value {
        Some(value) => Json(value).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(super) fn routes() -> Router<Arc<FakeGitHub>> {
    Router::new()
        .route(
            "/graphql",
            post(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 Json(body): Json<serde_json::Value>| async move {
                    let threads = fake.review_threads.lock().unwrap();
                    if body["variables"]["number"].is_u64() {
                        let mut nodes = threads.values().cloned().collect::<Vec<_>>();
                        nodes.sort_by_key(|thread| thread["id"].as_str().map(str::to_string));
                        return Json(serde_json::json!({ "data": { "repository": {
                            "pullRequest": { "reviewThreads": {
                                "pageInfo": { "hasNextPage": false, "endCursor": null },
                                "nodes": nodes,
                            } },
                        } } }));
                    }
                    let node_id = body["variables"]["id"].as_str().unwrap_or_default();
                    Json(serde_json::json!({ "data": { "node": threads.get(node_id).cloned() } }))
                },
            ),
        )
        .route(
            "/repos/{owner}/{repo}/pulls/{number}/reviews/{id}",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, _, id)): AxumPath<(String, String, u64, u64)>| async move {
                    found(fake.pull_reviews.lock().unwrap().get(&id).cloned())
                },
            ),
        )
        .route(
            "/repos/{owner}/{repo}/pulls/{number}/reviews/{id}/comments",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, _, id)): AxumPath<(String, String, u64, u64)>| async move {
                    let mut comments = fake
                        .review_comments
                        .lock()
                        .unwrap()
                        .values()
                        .filter(|comment| comment["pull_request_review_id"] == id)
                        .map(review_listing)
                        .collect::<Vec<_>>();
                    comments.sort_by_key(|comment| comment["id"].as_u64());
                    Json(comments)
                },
            ),
        )
        .route(
            "/repos/{owner}/{repo}/collaborators/{username}/permission",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, username)): AxumPath<(String, String, String)>| async move {
                    fake.permission_reads.fetch_add(1, Ordering::SeqCst);
                    match fake.collaborator_permissions.lock().unwrap().get(&username) {
                        Some(Some((permission, account_id))) => Json(serde_json::json!({
                            "permission": permission,
                            "role_name": permission,
                            "user": { "login": username, "id": account_id },
                        }))
                        .into_response(),
                        Some(None) => StatusCode::FORBIDDEN.into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                },
            ),
        )
        .route(
            "/repos/{owner}/{repo}/pulls/comments/{id}",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, id)): AxumPath<(String, String, u64)>| async move {
                    found(fake.review_comments.lock().unwrap().get(&id).cloned())
                },
            ),
        )
}
