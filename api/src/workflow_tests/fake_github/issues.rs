use super::{FakeGitHub, github_time};
use axum::{
    Json, Router,
    extract::{Query as AxumQuery, State as AxumState},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
pub(in crate::workflow_tests) struct FakeIssues {
    pub(in crate::workflow_tests) open: Mutex<Vec<serde_json::Value>>,
    comments: Mutex<HashMap<u64, Vec<serde_json::Value>>>,
    pub(in crate::workflow_tests) list_reads: AtomicUsize,
    pub(in crate::workflow_tests) comment_reads: AtomicUsize,
    pub(in crate::workflow_tests) list_rate_limited_until: Mutex<Option<u64>>,
    pub(in crate::workflow_tests) comments_unavailable_for: Mutex<Option<u64>>,
}

pub(in crate::workflow_tests) fn github_user(id: u64, login: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "login": login, "type": "User", "avatar_url": null })
}

pub(in crate::workflow_tests) fn issue(
    number: u64,
    title: &str,
    user: serde_json::Value,
    created_at_unix: u64,
) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "title": title,
        "body": format!("Body of issue {number}."),
        "user": user,
        "html_url": format!("https://github.com/{}/issues/{number}", super::GITHUB_FULL_NAME),
        "created_at": github_time(created_at_unix),
        "state": "open",
    })
}

pub(in crate::workflow_tests) fn pull_request(
    number: u64,
    created_at_unix: u64,
) -> serde_json::Value {
    let mut listed = issue(
        number,
        "A pull request",
        github_user(1, "octo-author"),
        created_at_unix,
    );
    listed["pull_request"] = serde_json::json!({ "url": "https://api.github.com/pulls" });
    listed
}

impl FakeIssues {
    pub(in crate::workflow_tests) fn report_comments(
        &self,
        issue_number: u64,
        comments: Vec<(u64, serde_json::Value, &str, u64)>,
    ) {
        let comments = comments
            .into_iter()
            .map(|(id, user, body, created_at_unix)| {
                serde_json::json!({
                    "id": id,
                    "user": user,
                    "body": body,
                    "html_url": format!(
                        "https://github.com/{}/issues/{issue_number}#issuecomment-{id}",
                        super::GITHUB_FULL_NAME
                    ),
                    "created_at": github_time(created_at_unix),
                })
            })
            .collect();
        self.comments.lock().unwrap().insert(issue_number, comments);
    }
}

fn page<T: Clone>(items: &[T], query: &HashMap<String, String>) -> Vec<T> {
    let page = query["page"].parse::<usize>().unwrap();
    let per_page = query["per_page"].parse::<usize>().unwrap();
    items
        .iter()
        .skip((page - 1) * per_page)
        .take(per_page)
        .cloned()
        .collect()
}

pub(super) fn routes() -> Router<Arc<FakeGitHub>> {
    Router::new().route(
        "/repos/{owner}/{name}/issues",
        get(
            |AxumState(fake): AxumState<Arc<FakeGitHub>>,
             AxumQuery(query): AxumQuery<HashMap<String, String>>| async move {
                assert_eq!(
                    (
                        query["state"].as_str(),
                        query["sort"].as_str(),
                        query["direction"].as_str()
                    ),
                    ("open", "created", "asc")
                );
                fake.issues.list_reads.fetch_add(1, Ordering::SeqCst);
                if let Some(reset) = *fake.issues.list_rate_limited_until.lock().unwrap() {
                    return (
                        StatusCode::FORBIDDEN,
                        [
                            ("x-ratelimit-remaining", "0".to_string()),
                            ("x-ratelimit-reset", reset.to_string()),
                        ],
                        Json(serde_json::json!({
                            "message": "API rate limit exceeded for installation ID 7."
                        })),
                    )
                        .into_response();
                }
                Json(page(&fake.issues.open.lock().unwrap(), &query)).into_response()
            },
        ),
    )
}

pub(super) fn comments_answer(
    fake: &FakeGitHub,
    number: u64,
    query: &HashMap<String, String>,
) -> Option<Response> {
    let reported = fake
        .issues
        .open
        .lock()
        .unwrap()
        .iter()
        .any(|issue| issue["number"] == number && issue.get("pull_request").is_none());
    if !reported {
        return None;
    }
    fake.issues.comment_reads.fetch_add(1, Ordering::SeqCst);
    if *fake.issues.comments_unavailable_for.lock().unwrap() == Some(number) {
        return Some(
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "message": "Server Error" })),
            )
                .into_response(),
        );
    }
    let comments = fake.issues.comments.lock().unwrap();
    Some(
        Json(page(
            comments.get(&number).map_or(&[][..], Vec::as_slice),
            query,
        ))
        .into_response(),
    )
}
