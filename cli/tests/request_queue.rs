mod support;

use axum::{
    Json, Router,
    extract::{OriginalUri, Query},
    http::StatusCode,
    routing::{get, put},
};
use scope_api_contract::{ErrorCode, ErrorResponse, RepositoryActor};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use support::*;

const OID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const LOADED_VERSION: u64 = 4;

#[test]
fn list_shows_the_queue_grouped_and_searches_on_the_server() {
    let dir = TempDir::new("request-queue-list");
    let server = QueueServer::start(None);

    let value = success(
        server
            .command(dir.path())
            .args(["--json", "--repo", "owner/repo", "request", "list"])
            .args(["--search", "fix", "--limit", "1"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["command"], "request.list");
    let rows = value["result"]["requests"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{value}");
    assert_eq!(rows[0]["section"], "active");
    assert_eq!(rows[0]["attention"]["group"], "needs_you");
    assert_eq!(rows[0]["request"]["name"], "fix-fresh");
    assert_eq!(rows[1]["attention"]["group"], "unclaimed");
    assert_eq!(
        server.seen(),
        [
            "GET /v1/repos/owner/repo/requests/queue?section=active&search=fix&limit=1",
            "GET /v1/repos/owner/repo/requests/queue?section=unclaimed&search=fix&limit=1",
        ]
    );

    let output = server
        .command(dir.path())
        .args(["--repo", "owner/repo", "request", "list"])
        .output()
        .unwrap();
    assert_success(&output, "show the request queue");
    let human = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Needs you",
        "fix-fresh (req_fresh) — fix-fresh · New reply or revision",
        "Waiting on others",
        "fix-claimed (req_claimed) — fix-claimed · Reviewing: @dana",
        "Unclaimed",
        "More: scope request list --section set-aside | done",
    ] {
        assert!(
            human.contains(expected),
            "missing {expected:?} in:\n{human}"
        );
    }

    let value = success(
        server
            .command(dir.path())
            .args(["--json", "--repo", "owner/repo", "request", "list"])
            .args(["--section", "set-aside"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        value["result"]["requests"][0]["attention"]["group"],
        "set_aside"
    );
    assert!(server.seen().contains(
        &"GET /v1/repos/owner/repo/requests/queue?section=set_aside&limit=30".to_string()
    ));

    scope_failure(
        dir.path(),
        ["request", "list", "--state", "open"],
        "unexpected argument '--state'",
    );
}

#[test]
fn snooze_sends_the_loaded_activity_version_and_a_future_time() {
    let dir = TempDir::new("request-queue-snooze");
    let server = QueueServer::start(None);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let value = success(
        server
            .command(dir.path())
            .args(["--json", "--repo", "owner/repo", "request", "snooze"])
            .args(["--request", "req_one", "--for", "tomorrow"])
            .output()
            .unwrap(),
    );

    assert_eq!(value["command"], "request.snooze");
    assert_eq!(value["result"]["request_id"], "req_one");
    let sent = server.attention_body();
    assert_eq!(sent["action"], "snooze");
    assert_eq!(sent["expected_activity_version"], LOADED_VERSION);
    let until = sent["until_unix"].as_u64().unwrap();
    assert!(until > now && until <= now + 2 * 24 * 60 * 60, "{sent}");
}

#[test]
fn a_stale_attention_change_shows_the_activity_that_moved_it() {
    let dir = TempDir::new("request-queue-stale");
    let server = QueueServer::start(Some(Moved::Revision));

    let output = server
        .command(dir.path())
        .args([
            "--repo",
            "owner/repo",
            "request",
            "claim",
            "--request",
            "req_one",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    for expected in [
        "request has newer activity; refresh before changing attention",
        "fix-one has newer activity since it was loaded.",
        "Revision pushed",
        "Run the command again to act on the current state.",
    ] {
        assert!(
            stderr.contains(expected),
            "missing {expected:?} in:\n{stderr}"
        );
    }
    assert_eq!(server.attention_body()["action"], "claim");
    assert_eq!(
        server.attention_body()["expected_activity_version"],
        LOADED_VERSION
    );
}

#[test]
fn a_request_closed_after_loading_reports_the_refusal_without_retry_advice() {
    let dir = TempDir::new("request-queue-closed");
    let server = QueueServer::start(Some(Moved::Closed));

    let output = server
        .command(dir.path())
        .args([
            "--repo",
            "owner/repo",
            "request",
            "settle",
            "--request",
            "req_one",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("attention actions require an open request"),
        "{stderr}"
    );
    assert!(!stderr.contains("Run the command again"), "{stderr}");
}

fn success(output: std::process::Output) -> Value {
    assert_success(&output, "run request queue command");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[derive(Clone, Copy)]
enum Moved {
    Revision,
    Closed,
}

impl Moved {
    fn state(self) -> &'static str {
        match self {
            Self::Revision => "Open",
            Self::Closed => "Closed",
        }
    }

    fn refusal(self) -> &'static str {
        match self {
            Self::Revision => "request has newer activity; refresh before changing attention",
            Self::Closed => "attention actions require an open request",
        }
    }
}

struct QueueServer {
    server: TestServer,
    seen: Arc<Mutex<Vec<String>>>,
    attention: Arc<Mutex<Value>>,
}

impl QueueServer {
    /// With `moved`, the request changes after the CLI loads it, so the server
    /// refuses the attention change.
    fn start(moved: Option<Moved>) -> Self {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let attention = Arc::new(Mutex::new(Value::Null));
        let detail_loads = Arc::new(AtomicUsize::new(0));
        let queue_seen = seen.clone();
        let attention_sent = attention.clone();
        let repo = repository_response(json!({
            "access": {"actor": serde_json::to_value(RepositoryActor::Member).unwrap()}
        }));
        let app = Router::new()
            .route(
                "/v1/session",
                get(|| async { Json(session_response("usr_test", "owner", "test@example.test")) }),
            )
            .route(
                "/v1/repos/owner/repo",
                get(move || {
                    let repo = repo.clone();
                    async move { Json(repo) }
                }),
            )
            .route(
                "/v1/repos/owner/repo/requests/queue",
                get(
                    move |OriginalUri(uri): OriginalUri,
                          Query(query): Query<HashMap<String, String>>| {
                        let seen = queue_seen.clone();
                        async move {
                            seen.lock().unwrap().push(format!("GET {uri}"));
                            Json(queue_page(
                                &query["section"],
                                query["limit"].parse().unwrap(),
                                query.contains_key("cursor"),
                            ))
                        }
                    },
                ),
            )
            .route(
                "/v1/repos/owner/repo/requests/req_one",
                get(move || {
                    let loads = detail_loads.fetch_add(1, Ordering::SeqCst);
                    let detail = match moved {
                        Some(moved) if loads > 0 => {
                            request_detail(LOADED_VERSION + 1, moved.state())
                        }
                        _ => request_detail(LOADED_VERSION, "Open"),
                    };
                    async move { Json(json!({"request": detail})) }
                }),
            )
            .route(
                "/v1/repos/owner/repo/requests/req_one/attention",
                put(move |Json(body): Json<Value>| {
                    *attention_sent.lock().unwrap() = body;
                    async move {
                        match moved {
                            Some(moved) => {
                                let refusal =
                                    ErrorResponse::new(ErrorCode::Conflict, moved.refusal());
                                (
                                    StatusCode::CONFLICT,
                                    Json(serde_json::to_value(refusal).unwrap()),
                                )
                            }
                            None => (StatusCode::OK, Json(attention_mutation())),
                        }
                    }
                }),
            )
            .route(
                "/v1/repos/owner/repo/requests/req_one/activity",
                get(|| async {
                    Json(json!({
                        "events": [{
                            "id": "event_5", "position": LOADED_VERSION + 1,
                            "actor": {"id": "usr_sam", "handle": "sam"},
                            "kind": "RevisionPushed",
                            "payload": {"RevisionPushed": {
                                "old_head_oid": OID,
                                "new_head_oid": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                                "note": null
                            }},
                            "created_at_unix": 50
                        }],
                        "through_position": LOADED_VERSION + 1
                    }))
                }),
            );
        Self {
            server: TestServer::new(app),
            seen,
            attention,
        }
    }

    fn command(&self, cwd: &std::path::Path) -> std::process::Command {
        self.server.command(cwd)
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    fn attention_body(&self) -> Value {
        self.attention.lock().unwrap().clone()
    }
}

/// Every section has a second, empty page, so paging stops only at `--limit`
/// or when the server runs out. Like the server, a page holds at most `limit` rows.
fn queue_page(section: &str, limit: usize, next_page: bool) -> Value {
    if next_page {
        return json!({"requests": [], "next_cursor": null, "next_attention_at_unix": null});
    }
    let items = match section {
        "active" => vec![
            queue_item("req_fresh", "fix-fresh", "needs_you", "new_activity", None),
            queue_item(
                "req_claimed",
                "fix-claimed",
                "waiting",
                "claimed_elsewhere",
                Some("dana"),
            ),
        ],
        "unclaimed" => vec![queue_item(
            "req_open",
            "fix-open",
            "unclaimed",
            "unclaimed",
            None,
        )],
        "set_aside" => vec![queue_item(
            "req_waiting",
            "fix-waiting",
            "set_aside",
            "waiting",
            None,
        )],
        _ => Vec::new(),
    };
    let items: Vec<Value> = items.into_iter().take(limit).collect();
    json!({"requests": items, "next_cursor": "next", "next_attention_at_unix": null})
}

fn queue_item(id: &str, name: &str, group: &str, reason: &str, claimer: Option<&str>) -> Value {
    json!({
        "attention_at_unix": 1,
        "request": {
            "id": id, "name": name, "title": name, "author_role": "Public",
            "audience": "Public", "head_oid": OID, "state": "Open",
            "submitted_at_unix": 1, "updated_at_unix": 2,
            "mergeability": {"status": "Ready", "current_main_oid": OID, "request_head_oid": OID, "reason": null}
        },
        "author": {"id": "usr_author", "handle": "author"},
        "attention": attention(group, reason),
        "claimer": claimer.map(|handle| json!({"id": format!("usr_{handle}"), "handle": handle}))
    })
}

fn attention(group: &str, reason: &str) -> Value {
    json!({
        "group": group, "state": "active", "reason": reason, "activity_version": LOADED_VERSION,
        "through_activity_version": LOADED_VERSION, "snoozed_until_unix": null, "revision": 1,
        "can_claim": true, "can_set_aside": true, "can_restore": false, "can_release": false
    })
}

fn attention_mutation() -> Value {
    let mut attention = attention("set_aside", "snoozed");
    attention["state"] = json!("snoozed");
    attention["snoozed_until_unix"] = json!(4_000_000_000_u64);
    json!({"attention": attention, "claimer": null})
}

fn request_detail(activity_version: u64, state: &str) -> Value {
    json!({"id":"req_one","name":"fix-one","title":"Fix one","description_markdown":"","author_user_id":"usr_author","author_role":"Public","audience":"Public","base_main_oid":OID,"head_oid":OID,"state":state,"activity_version":activity_version,"submitted_at_unix":1,"closed_at_unix":null,"closed_by_user_id":null,"merged_at_unix":null,"merged_by_user_id":null,"merged_head_oid":null,"merged_main_oid":null,"created_at_unix":1,"updated_at_unix":2,"invitees":[],"permissions":{"can_view_activity":true,"can_open_discussion":true,"can_reply_to_discussion":true,"can_wait_after_reply":false,"can_edit_identity":false,"can_pull_branch":true,"can_push_branch":false,"can_submit":false,"can_manage_invitees":false,"can_leave_request":false,"can_close":true,"can_merge":true},"mergeability":{"status":"Ready","current_main_oid":OID,"request_head_oid":OID,"reason":null}})
}
