use super::FakeGitHub;
use axum::{
    Json, Router,
    extract::{Path as AxumPath, State as AxumState},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use std::sync::Arc;

impl FakeGitHub {
    pub(in crate::workflow_tests) fn report_pull_review(
        &self,
        review: serde_json::Value,
        comments: Vec<serde_json::Value>,
    ) {
        let mut reported = self.review_comments.lock().unwrap();
        for comment in &comments {
            reported.insert(comment["id"].as_u64().unwrap(), comment.clone());
        }
        self.pull_reviews
            .lock()
            .unwrap()
            .insert(review["id"].as_u64().unwrap(), (review, comments));
    }
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
            "/repos/{owner}/{repo}/pulls/{number}/reviews/{id}",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, _, id)): AxumPath<(String, String, u64, u64)>| async move {
                    found(
                        fake.pull_reviews
                            .lock()
                            .unwrap()
                            .get(&id)
                            .map(|(review, _)| review.clone()),
                    )
                },
            ),
        )
        .route(
            "/repos/{owner}/{repo}/pulls/{number}/reviews/{id}/comments",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, _, id)): AxumPath<(String, String, u64, u64)>| async move {
                    found(
                        fake.pull_reviews
                            .lock()
                            .unwrap()
                            .get(&id)
                            .map(|(_, comments)| serde_json::Value::from(comments.clone())),
                    )
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
