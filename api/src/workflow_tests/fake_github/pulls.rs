use super::{FakeGitHub, GITHUB_FULL_NAME};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query as AxumQuery, State as AxumState},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

const APP_LOGIN: &str = "scope-checks[bot]";

type LookupHook = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

#[derive(Default)]
pub(in crate::workflow_tests) struct FakePulls {
    pub(in crate::workflow_tests) pulls: Mutex<Vec<serde_json::Value>>,
    pub(in crate::workflow_tests) comments: Mutex<Vec<(u64, String)>>,
    bot_comments: Mutex<HashMap<(u64, String), String>>,
    reactions: Mutex<Vec<(u64, serde_json::Value)>>,
    reviews: Mutex<Vec<(u64, serde_json::Value)>>,
    pub(in crate::workflow_tests) forbidden: AtomicBool,
    pub(in crate::workflow_tests) rate_limited_once: AtomicBool,
    pub(in crate::workflow_tests) writes_unreachable: AtomicBool,
    pub(in crate::workflow_tests) before_lookup: Mutex<Option<LookupHook>>,
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
            "closed_by": null,
        }));
        number
    }

    pub(in crate::workflow_tests) fn close_as(&self, number: u64, login: &str) {
        let mut pulls = self.pulls.lock().unwrap();
        let pull = &mut pulls[number as usize - 1];
        pull["state"] = "closed".into();
        pull["closed_by"] = serde_json::json!({ "login": login });
    }

    pub(in crate::workflow_tests) fn comment_as_app(&self, number: u64, body: &str) {
        self.comments
            .lock()
            .unwrap()
            .push((number, body.to_string()));
    }

    pub(in crate::workflow_tests) fn comment_as(&self, number: u64, login: &str, body: &str) {
        self.bot_comments
            .lock()
            .unwrap()
            .insert((number, login.to_string()), body.to_string());
    }

    pub(in crate::workflow_tests) fn react_as(
        &self,
        number: u64,
        login: &str,
        content: &str,
        created_at: &str,
    ) {
        self.reactions.lock().unwrap().push((
            number,
            serde_json::json!({
                "user": { "login": login },
                "content": content,
                "created_at": created_at,
            }),
        ));
    }

    pub(in crate::workflow_tests) fn review_as(
        &self,
        number: u64,
        review_id: u64,
        login: &str,
        commit: &str,
    ) {
        self.reviews.lock().unwrap().push((
            number,
            serde_json::json!({ "id": review_id, "user": { "login": login }, "commit_id": commit }),
        ));
    }

    fn listed(
        items: &Mutex<Vec<(u64, serde_json::Value)>>,
        number: u64,
        query: &HashMap<String, String>,
    ) -> Vec<serde_json::Value> {
        if query.get("page").map(String::as_str) != Some("1") {
            return Vec::new();
        }
        items
            .lock()
            .unwrap()
            .iter()
            .filter(|(on, _)| *on == number)
            .map(|(_, item)| item.clone())
            .collect()
    }

    pub(in crate::workflow_tests) fn pull(&self, number: u64) -> serde_json::Value {
        self.pulls.lock().unwrap()[number as usize - 1].clone()
    }

    fn refused_write(&self) -> Option<axum::response::Response> {
        if self.writes_unreachable.load(Ordering::SeqCst) {
            return Some(StatusCode::NOT_FOUND.into_response());
        }
        self.refused()
    }

    fn refused(&self) -> Option<axum::response::Response> {
        if self.rate_limited_once.swap(false, Ordering::SeqCst) {
            return Some(
                (
                    StatusCode::FORBIDDEN,
                    [
                        ("x-ratelimit-remaining", "0"),
                        ("x-ratelimit-reset", "4102444800"),
                    ],
                    Json(serde_json::json!({ "message": "API rate limit exceeded" })),
                )
                    .into_response(),
            );
        }
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
            "/repos/{owner}/{name}/pulls/{number}/reviews",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, number)): AxumPath<(String, String, u64)>,
                 AxumQuery(query): AxumQuery<HashMap<String, String>>| async move {
                    if let Some(refused) = fake.pulls.refused() {
                        return refused;
                    }
                    Json(FakePulls::listed(&fake.pulls.reviews, number, &query)).into_response()
                },
            ),
        )
        .route(
            "/repos/{owner}/{name}/issues/{number}/reactions",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, number)): AxumPath<(String, String, u64)>,
                 AxumQuery(query): AxumQuery<HashMap<String, String>>| async move {
                    if let Some(refused) = fake.pulls.refused() {
                        return refused;
                    }
                    Json(FakePulls::listed(&fake.pulls.reactions, number, &query)).into_response()
                },
            ),
        )
        .route(
            "/repos/{owner}/{name}/pulls",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumQuery(query): AxumQuery<HashMap<String, String>>| async move {
                    let hook = fake.pulls.before_lookup.lock().unwrap().take();
                    if let Some(hook) = hook {
                        hook().await;
                    }
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
                    if let Some(refused) = fake.pulls.refused_write() {
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
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, number)): AxumPath<(String, String, u64)>| async move {
                    if let Some(refused) = fake.pulls.refused() {
                        return refused;
                    }
                    Json(fake.pulls.pull(number)).into_response()
                },
            )
            .patch(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, number)): AxumPath<(String, String, u64)>,
                 Json(body): Json<serde_json::Value>| async move {
                    if let Some(refused) = fake.pulls.refused_write() {
                        return refused;
                    }
                    let mut pulls = fake.pulls.pulls.lock().unwrap();
                    let pull = &mut pulls[number as usize - 1];
                    if let Some(state) = body.get("state") {
                        pull["state"] = state.clone();
                        pull["closed_by"] = serde_json::json!({ "login": APP_LOGIN });
                    }
                    if let Some(base) = body.get("base") {
                        pull["base"]["ref"] = base.clone();
                    }
                    for field in ["title", "body"] {
                        if let Some(value) = body.get(field) {
                            pull[field] = value.clone();
                        }
                    }
                    Json(pull.clone()).into_response()
                },
            ),
        )
        .route(
            "/repos/{owner}/{name}/issues/{number}",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, number)): AxumPath<(String, String, u64)>| async move {
                    if let Some(refused) = fake.pulls.refused() {
                        return refused;
                    }
                    let pull = fake.pulls.pull(number);
                    Json(serde_json::json!({
                        "number": number, "state": pull["state"], "closed_by": pull["closed_by"],
                    }))
                    .into_response()
                },
            ),
        )
        .route(
            "/repos/{owner}/{name}/issues/{number}/comments",
            get(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, number)): AxumPath<(String, String, u64)>,
                 AxumQuery(query): AxumQuery<HashMap<String, String>>| async move {
                    if let Some(refused) = fake.pulls.refused() {
                        return refused;
                    }
                    let comments = if query["page"] == "1" {
                        fake.pulls
                            .comments
                            .lock()
                            .unwrap()
                            .iter()
                            .filter(|(on, _)| *on == number)
                            .map(|(_, body)| {
                                serde_json::json!({ "body": body, "user": { "login": APP_LOGIN } })
                            })
                            .chain(
                                fake.pulls
                                    .bot_comments
                                    .lock()
                                    .unwrap()
                                    .iter()
                                    .filter(|((on, _), _)| *on == number)
                                    .map(|((_, login), body)| {
                                        serde_json::json!({ "body": body, "user": { "login": login } })
                                    }),
                            )
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    Json(comments).into_response()
                },
            )
            .post(
                |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                 AxumPath((_, _, number)): AxumPath<(String, String, u64)>,
                 Json(body): Json<serde_json::Value>| async move {
                    if let Some(refused) = fake.pulls.refused_write() {
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
