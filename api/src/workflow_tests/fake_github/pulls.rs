use super::{FakeGitHub, GITHUB_FULL_NAME};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query as AxumQuery, State as AxumState},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch, post},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
pub(in crate::workflow_tests) struct FakePulls {
    pub(in crate::workflow_tests) pulls: Mutex<Vec<serde_json::Value>>,
    pub(in crate::workflow_tests) comments: Mutex<Vec<(u64, String)>>,
    pub(in crate::workflow_tests) forbidden: AtomicBool,
}

impl FakePulls {
    pub(in crate::workflow_tests) fn open(&self, head: &str, base: &str) -> u64 {
        let mut pulls = self.pulls.lock().unwrap();
        let number = pulls.len() as u64 + 1;
        pulls.push(serde_json::json!({
            "number": number,
            "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/{number}"),
            "state": "open",
            "title": "",
            "body": "",
            "draft": false,
            "head": { "ref": head },
            "base": { "ref": base },
        }));
        number
    }

    pub(in crate::workflow_tests) fn pull(&self, number: u64) -> serde_json::Value {
        self.pulls.lock().unwrap()[number as usize - 1].clone()
    }

    fn refused(&self) -> Option<axum::response::Response> {
        self.forbidden.load(Ordering::SeqCst).then(|| {
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "message": "Resource not accessible by integration" })),
            )
                .into_response()
        })
    }
}

pub(super) fn routes() -> Router<Arc<FakeGitHub>> {
    Router::new()
        .route(
            "/repos/{owner}/{name}/pulls",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumQuery(query): AxumQuery<HashMap<String, String>>| async move {
                    if let Some(refused) = fake.pulls.refused() {
                        return refused;
                    }
                    let owner = GITHUB_FULL_NAME.split_once('/').unwrap().0;
                    let open = fake
                        .pulls
                        .pulls
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|pull| {
                            pull["state"] == query["state"].as_str()
                                && format!("{owner}:{}", pull["head"]["ref"].as_str().unwrap())
                                    == query["head"]
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    Json(open).into_response()
                },
            )
            .post(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 Json(body): Json<serde_json::Value>| async move {
                    if let Some(refused) = fake.pulls.refused() {
                        return refused;
                    }
                    let number = fake.pulls.open(
                        body["head"].as_str().unwrap(),
                        body["base"].as_str().unwrap(),
                    );
                    let mut pulls = fake.pulls.pulls.lock().unwrap();
                    let pull = &mut pulls[number as usize - 1];
                    pull["title"] = body["title"].clone();
                    pull["body"] = body["body"].clone();
                    pull["draft"] = body["draft"].clone();
                    (StatusCode::CREATED, Json(pull.clone())).into_response()
                },
            ),
        )
        .route(
            "/repos/{owner}/{name}/pulls/{number}",
            patch(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, number)): AxumPath<(String, String, u64)>,
                 Json(body): Json<serde_json::Value>| async move {
                    if let Some(refused) = fake.pulls.refused() {
                        return refused;
                    }
                    let mut pulls = fake.pulls.pulls.lock().unwrap();
                    let pull = &mut pulls[number as usize - 1];
                    if let Some(state) = body.get("state") {
                        pull["state"] = state.clone();
                    }
                    if let Some(base) = body.get("base") {
                        pull["base"]["ref"] = base.clone();
                    }
                    Json(pull.clone()).into_response()
                },
            ),
        )
        .route(
            "/repos/{owner}/{name}/issues/{number}/comments",
            post(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, number)): AxumPath<(String, String, u64)>,
                 Json(body): Json<serde_json::Value>| async move {
                    if let Some(refused) = fake.pulls.refused() {
                        return refused;
                    }
                    fake.pulls
                        .comments
                        .lock()
                        .unwrap()
                        .push((number, body["body"].as_str().unwrap().to_string()));
                    (StatusCode::CREATED, Json(serde_json::json!({ "id": 1 }))).into_response()
                },
            ),
        )
}
