mod support;
use axum::{Json, Router, extract::Query, response::IntoResponse, routing::get};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use support::*;

fn run(state: &str) -> serde_json::Value {
    serde_json::json!({"id":"run-1","repository_id":"owner/repo","workflow_name":"checks","git_oid":"1234567890","state":state,"cancellation_requested":false,"logs_truncated":false,"created_at_unix":1,"updated_at_unix":1,"completed_at_unix":null})
}
fn log(position: u64) -> serde_json::Value {
    serde_json::json!({"attempt_id":"attempt-1","job_key":"build","step_index":0,"position":position,"sequence":position,"text":format!("line {position}\n"),"created_at_unix":1})
}
fn event(name: &str, payload: serde_json::Value) -> String {
    format!("event: {name}\ndata: {payload}\n\n")
}

struct Server(TestServer);
impl Server {
    fn new(router: Router) -> Self {
        let router = router.route(
            "/v1/session",
            get(|| async {
                Json(support::session_response(
                    "user-test",
                    "owner",
                    "owner@example.test",
                ))
            }),
        );
        Self(TestServer::new(router))
    }
    fn command(&self, dir: &TempDir) -> std::process::Command {
        let mut command = self.0.command(dir.path());
        command.args(["--json", "--repo", "owner/repo"]);
        command
    }
}

#[test]
fn run_discovery_and_history_work_without_a_checkout_and_preserve_pagination() {
    let server = Server::new(Router::new()
        .route("/v1/repos/owner/repo/run-workflows", get(|| async { Json(serde_json::json!({"workflows":[{"key":"checks","name":"Checks","path":"/.scope/runs/checks.yml","manual":true,"push_main":true,"job_count":1}]})) }))
        .route("/v1/repos/owner/repo/runs", get(|Query(query): Query<HashMap<String,String>>| async move {
            assert_eq!(query.get("workflow").unwrap(), "checks"); assert_eq!(query.get("limit").unwrap(), "2"); assert_eq!(query.get("after").unwrap(), "v2:99:checks");
            Json(serde_json::json!({"runs":[],"next_cursor":"v2:42:checks"}))
        })));
    let dir = TempDir::new("run-remote");
    for args in [
        vec!["run", "workflows"],
        vec![
            "run",
            "list",
            "--workflow",
            "checks",
            "--limit",
            "2",
            "--after",
            "v2:99:checks",
        ],
    ] {
        let output = server.command(&dir).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let data: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(data["command"].as_str().unwrap().starts_with("run."));
    }
}

#[test]
fn run_watch_reconnects_with_cursor_deduplicates_and_reports_failure() {
    let cursors = Arc::new(Mutex::new(Vec::new()));
    let received = cursors.clone();
    let server = Server::new(Router::new().route(
        "/v1/repos/owner/repo/runs/run-1/events",
        get(move |Query(query): Query<HashMap<String, u64>>| {
            let received = received.clone();
            async move {
                let after = query["after"];
                received.lock().unwrap().push(after);
                let body = if after == 0 {
                    event("log", log(1))
                } else {
                    event("log", log(1)) + &event("log", log(2)) + &event("status", run("failed"))
                };
                ([("content-type", "text/event-stream")], body).into_response()
            }
        }),
    ));
    let dir = TempDir::new("run-stream");
    let output = server
        .command(&dir)
        .args(["run", "watch", "run-1", "--timeout", "10"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(*cursors.lock().unwrap(), [0, 1]);
    assert_eq!(
        lines.iter().filter(|v| v["command"] == "run.log").count(),
        2
    );
    assert_eq!(lines[2]["command"], "run.status");
}

#[test]
fn run_watch_timeout_returns_a_resumable_temporary_error() {
    let server = Server::new(Router::new().route(
        "/v1/repos/owner/repo/runs/run-1/events",
        get(|| async { ([("content-type", "text/event-stream")], ": keepalive\n\n") }),
    ));
    let dir = TempDir::new("run-timeout");
    let output = server
        .command(&dir)
        .args(["run", "watch", "run-1", "--timeout", "1", "--after", "8"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(6));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--after 8"));
}

#[test]
fn stored_logs_filter_jobs_and_follow_each_step_cursor() {
    let summary = serde_json::json!({"id":"run-1","workflow_name":"checks","git_oid":"1234567890","trigger":"manual","state":"succeeded","cancellation_requested":false,"created_at_unix":1,"updated_at_unix":1,"completed_at_unix":1,"can_cancel":false,"can_retry":true});
    let job = |key: &str| {
        serde_json::json!({
            "job":{"key":key,"needs":[],"pinned_container_image":"example/image@sha256:abc","state":"succeeded","created_at_unix":1,"started_at_unix":1,"updated_at_unix":1,"completed_at_unix":1},
            "attempts":[{"id":format!("attempt-{key}"),"number":1,"external_run_id":null,"runtime_version":"1","state":"succeeded","created_at_unix":1,"started_at_unix":1,"completed_at_unix":1,"terminal_reason":null,"cache_setup":null,"caches":[],"steps":[{"index":0,"name":"test","command":"test","state":"succeeded","started_at_unix":1,"completed_at_unix":1,"exit_code":0}]}]
        })
    };
    let detail = serde_json::json!({"run":summary,"jobs":[job("build"),job("ignored")]});
    let cursors = Arc::new(Mutex::new(Vec::new()));
    let received = cursors.clone();
    let server = Server::new(Router::new()
        .route("/v1/repos/owner/repo/runs/run-1/detail", get(move || {let detail = detail.clone(); async move {Json(detail)}}))
        .route("/v1/repos/owner/repo/runs/run-1/attempts/attempt-build/steps/0/logs", get(move |Query(query): Query<HashMap<String,u64>>| {
            let received = received.clone(); async move {
                let after = query["after"]; received.lock().unwrap().push(after);
                let position = after + 1;
                Json(serde_json::json!({"logs":[{"byte_length":7,"position":position,"sequence":position,"text":format!("line {position}\n"),"created_at_unix":1}],"next_after":position,"has_more":after == 0,"has_earlier":false,"logs_truncated":false}))
            }
        })));
    let dir = TempDir::new("run-logs");
    let output = server
        .command(&dir)
        .args(["run", "logs", "run-1", "--job", "build"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["command"], "run.logs");
    assert_eq!(*cursors.lock().unwrap(), [0, 1]);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("line 1") && text.contains("line 2"), "{text}");
    assert!(!text.contains("ignored"), "{text}");
}
