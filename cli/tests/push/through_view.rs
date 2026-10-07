use super::support::*;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{OriginalUri, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
struct FakeScope {
    git_root: PathBuf,
    access_view: &'static str,
    request_list_fails: bool,
    seen: Arc<Mutex<Vec<String>>>,
    intents: Arc<Mutex<Vec<Value>>>,
}

#[test]
fn a_narrower_member_pushes_main_through_their_view_and_lands_as_a_request() {
    let workspace = TempDir::new("narrower-main-push");
    let (scope, server) = start_fake_scope(&workspace, "agent");
    let checkout = checkout_with_new_commit(&workspace, &server, "agent");
    let head = git_stdout(&checkout, ["rev-parse", "HEAD"]);

    let output = server
        .command(&checkout)
        .args(["push", "--main"])
        .output()
        .unwrap();

    assert_success(&output, "push main through the agent view");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains(&format!(
            "Landed as request main-push-{}-2 (req_landed) in the Agent view",
            &head[..12]
        )),
        "{stdout}"
    );
    let intents = scope.intents.lock().unwrap();
    assert_eq!(intents.len(), 1);
    assert_eq!(intents[0]["view"], "agent");
    assert_eq!(intents[0]["head_oid"], head);
    let seen = scope.seen.lock().unwrap();
    assert!(
        seen.iter()
            .any(|entry| entry == "POST /git/agent/owner/repo/git-receive-pack intent-token"),
        "{seen:?}"
    );
    assert!(!seen.iter().any(|entry| entry.contains("/git/private/")));
    assert_eq!(
        seen.iter()
            .filter(|entry| entry.starts_with("GET /v1/repos/owner/repo/requests"))
            .collect::<Vec<_>>(),
        [
            "GET /v1/repos/owner/repo/requests",
            "GET /v1/repos/owner/repo/requests page-2",
        ]
    );
    assert!(!seen.iter().any(|entry| entry.ends_with("/config")));
    assert_eq!(
        git_stdout(
            &scope.git_root.join("owner/repo"),
            ["rev-parse", "refs/heads/main"]
        ),
        head
    );
}

#[test]
fn pushing_the_view_main_again_lands_nothing_and_invents_no_request() {
    let workspace = TempDir::new("unchanged-main-push");
    let (scope, server) = start_fake_scope(&workspace, "agent");
    let checkout = checkout_at_view_main(&workspace, &server, "agent");
    let head = git_stdout(&checkout, ["rev-parse", "HEAD"]);

    let human = server
        .command(&checkout)
        .args(["push", "--main"])
        .output()
        .unwrap();
    assert_success(&human, "push an unchanged main through the agent view");
    let stdout = String::from_utf8(human.stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        format!(
            "The Agent view's main is already at {}; nothing to push",
            &head[..7]
        )
    );

    let json_output = server
        .command(&checkout)
        .args(["--json", "push", "--main"])
        .output()
        .unwrap();
    assert_success(&json_output, "push an unchanged main as JSON");
    let envelope: Value = serde_json::from_slice(&json_output.stdout).unwrap();
    assert_eq!(envelope["result"]["landed"], false, "{envelope}");
    assert_eq!(envelope["result"]["request"], Value::Null, "{envelope}");
    assert_eq!(envelope["result"]["commit"], head, "{envelope}");

    assert_eq!(scope.intents.lock().unwrap().len(), 2);
    let seen = scope.seen.lock().unwrap();
    assert!(
        !seen.iter().any(|entry| entry.contains("git-receive-pack")
            || entry.starts_with("GET /v1/repos/owner/repo/requests")),
        "{seen:?}"
    );
}

#[test]
fn a_landed_push_whose_request_lookup_fails_says_not_to_push_again() {
    let workspace = TempDir::new("landed-lookup-failure");
    let (scope, server) = start_fake_scope_with(&workspace, "agent", true);
    let checkout = checkout_with_new_commit(&workspace, &server, "agent");
    let head = git_stdout(&checkout, ["rev-parse", "HEAD"]);

    let output = server
        .command(&checkout)
        .args(["--json", "push", "--main"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(5), "{output:?}");
    let failure: Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert!(
        failure["message"]
            .as_str()
            .unwrap()
            .starts_with("the push to owner/repo landed as a request in the agent view"),
        "{failure}"
    );
    assert_eq!(failure["recovery"]["landed"], true, "{failure}");
    assert_eq!(failure["recovery"]["commit"], head, "{failure}");
    assert_eq!(failure["recovery"]["view"], "agent", "{failure}");
    assert_eq!(
        failure["recovery"]["recovery_commands"],
        json!([["scope", "request", "list"]])
    );
    assert_eq!(
        git_stdout(
            &scope.git_root.join("owner/repo"),
            ["rev-parse", "refs/heads/main"]
        ),
        head
    );
}

#[test]
fn a_full_view_member_cannot_push_main_through_a_narrower_remote() {
    let workspace = TempDir::new("full-member-narrower-remote");
    let (scope, server) = start_fake_scope(&workspace, "private");
    let checkout = checkout_with_new_commit(&workspace, &server, "agent");

    let output = server
        .command(&checkout)
        .args(["push", "--main"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains(&format!(
            "your pushes to main go through the Private view, but remote scope reads the Agent view; push from a remote on {}/git/private/owner/repo",
            server.api_url
        )),
        "{stderr}"
    );
    assert!(scope.intents.lock().unwrap().is_empty());
}

fn start_fake_scope(workspace: &TempDir, access_view: &'static str) -> (FakeScope, TestServer) {
    start_fake_scope_with(workspace, access_view, false)
}

fn start_fake_scope_with(
    workspace: &TempDir,
    access_view: &'static str,
    request_list_fails: bool,
) -> (FakeScope, TestServer) {
    let git_root = workspace.path().join("server");
    let bare = git_root.join("owner/repo");
    std::fs::create_dir_all(&bare).unwrap();
    run_git(&bare, ["init", "--bare", "--initial-branch=main"]);
    let scope = FakeScope {
        git_root,
        access_view,
        request_list_fails,
        seen: Arc::default(),
        intents: Arc::default(),
    };
    let server = TestServer::with_url(|api_url| {
        let api_url = api_url.to_string();
        Router::new()
            .route(
                "/v1/session",
                get(|| async { Json(session_response("usr_agent", "agent", "agent@example.test")) }),
            )
            .route(
                "/v1/repos/owner/repo",
                get(move |State(scope): State<FakeScope>| {
                    let api_url = api_url.clone();
                    async move {
                    Json(repository_response(json!({
                        "git_remote_url": format!("{api_url}/git/{}/owner/repo", scope.access_view),
                        "access": {"actor": "Member", "view": scope.access_view, "can_push": true},
                        "views": [
                            {"id": "public", "name": "Public", "includes": [], "readers": "anyone"},
                            {"id": "private", "name": "Private", "includes": "all", "readers": "assigned"},
                            {"id": "agent", "name": "Agent", "includes": ["public"], "readers": "assigned"},
                        ],
                    })))
                    }
                }),
            )
            .route("/v1/repos/owner/repo/config", get(forbidden_config))
            .route("/v1/repos/owner/repo/push-intents", post(create_intent))
            .route("/v1/repos/owner/repo/requests", get(list_requests))
            .route("/git/{*path}", any(git_http_backend))
            .with_state(scope.clone())
    });
    (scope, server)
}

async fn forbidden_config(State(scope): State<FakeScope>) -> Response {
    scope
        .seen
        .lock()
        .unwrap()
        .push("GET /v1/repos/owner/repo/config".into());
    (
        StatusCode::FORBIDDEN,
        Json(json!({"code": "forbidden", "message": "the repository's full view is required"})),
    )
        .into_response()
}

async fn create_intent(State(scope): State<FakeScope>, Json(body): Json<Value>) -> Json<Value> {
    scope.intents.lock().unwrap().push(body);
    let expires_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    let view_main = git_stdout(
        &scope.git_root.join("owner/repo"),
        ["rev-parse", "refs/heads/main"],
    );
    Json(json!({
        "token": "intent-token",
        "base_head_oid": view_main,
        "expires_at_unix": expires_at_unix,
        "lands_as_request": true,
    }))
}

async fn list_requests(
    State(scope): State<FakeScope>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let cursor = query.get("cursor").cloned();
    scope.seen.lock().unwrap().push(
        format!(
            "GET /v1/repos/owner/repo/requests {}",
            cursor.clone().unwrap_or_default()
        )
        .trim_end()
        .to_string(),
    );
    if scope.request_list_fails {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"code": "internal", "message": "request list unavailable"})),
        )
            .into_response();
    }
    let head = git_stdout(
        &scope.git_root.join("owner/repo"),
        ["rev-parse", "refs/heads/main"],
    );
    let short = &head[..12];
    let other_head = "1".repeat(40);
    match cursor.as_deref() {
        None => Json(json!({
            "requests": [
                list_item("req_closed", &format!("main-push-{short}"), "usr_agent", "agent", &head, "Closed"),
                list_item("req_other_author", &format!("main-push-{short}-3"), "usr_other", "agent", &head, "Open"),
                list_item("req_other_head", &format!("main-push-{short}-4"), "usr_agent", "agent", &other_head, "Open"),
                list_item("req_other_view", &format!("main-push-{short}-5"), "usr_agent", "public", &head, "Open"),
            ],
            "next_cursor": "page-2",
        }))
        .into_response(),
        Some(_) => Json(json!({
            "requests": [
                list_item("req_landed", &format!("main-push-{short}-2"), "usr_agent", "agent", &head, "Open"),
                list_item("req_branch", "agent-feature", "usr_agent", "agent", &head, "Open"),
            ],
            "next_cursor": null,
        }))
        .into_response(),
    }
}

fn list_item(id: &str, name: &str, author: &str, view: &str, head: &str, state: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "title": "Main push from agent",
        "author_user_id": author,
        "author_role": "Member",
        "view": view,
        "head_oid": head,
        "state": state,
        "submitted_at_unix": 1,
        "updated_at_unix": 2,
        "mergeability": {
            "status": "ChecksPending",
            "current_main_oid": head,
            "request_head_oid": head,
            "reason": null,
        },
    })
}

async fn git_http_backend(
    State(scope): State<FakeScope>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    let intent = header("x-scope-push-intent");
    scope.seen.lock().unwrap().push(
        format!("{method} {} {intent}", uri.path())
            .trim_end()
            .to_string(),
    );
    let path_info = uri
        .path()
        .splitn(4, '/')
        .nth(3)
        .map(|repository_path| format!("/{repository_path}"))
        .unwrap();
    let environment = [
        ("GIT_PROJECT_ROOT", scope.git_root.display().to_string()),
        ("GIT_HTTP_EXPORT_ALL", "1".into()),
        ("REMOTE_USER", "agent".into()),
        ("PATH_INFO", path_info),
        ("REQUEST_METHOD", method.to_string()),
        ("QUERY_STRING", uri.query().unwrap_or_default().into()),
        ("CONTENT_TYPE", header("content-type")),
        ("CONTENT_LENGTH", body.len().to_string()),
        ("HTTP_CONTENT_ENCODING", header("content-encoding")),
        ("GIT_PROTOCOL", header("git-protocol")),
    ];
    let output = tokio::task::spawn_blocking(move || run_http_backend(&environment, &body))
        .await
        .unwrap();
    cgi_response(&output)
}

fn run_http_backend(environment: &[(&str, String)], body: &[u8]) -> Vec<u8> {
    let mut child = Command::new("git")
        .arg("http-backend")
        .envs(environment.iter().map(|(key, value)| (*key, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(body).unwrap();
    child.wait_with_output().unwrap().stdout
}

fn cgi_response(output: &[u8]) -> Response {
    let split = output
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let mut response = Response::builder();
    for line in String::from_utf8_lossy(&output[..split]).lines() {
        let (name, value) = line.split_once(": ").unwrap();
        response = if name.eq_ignore_ascii_case("status") {
            response.status(value[..3].parse::<u16>().unwrap())
        } else {
            response.header(name, value)
        };
    }
    response
        .body(axum::body::Body::from(output[split + 4..].to_vec()))
        .unwrap()
}

fn checkout_with_new_commit(workspace: &TempDir, server: &TestServer, view: &str) -> PathBuf {
    let checkout = checkout_at_view_main(workspace, server, view);
    std::fs::write(checkout.join("agent.md"), "agent change\n").unwrap();
    run_git(&checkout, ["add", "agent.md"]);
    commit_all(&checkout, "Agent change");
    checkout
}

fn checkout_at_view_main(workspace: &TempDir, server: &TestServer, view: &str) -> PathBuf {
    let checkout = workspace.path().join("checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    create_repo_with_head(&checkout);
    let bare = workspace.path().join("server/owner/repo");
    run_git(
        &checkout,
        ["push", bare.to_str().unwrap(), "HEAD:refs/heads/main"],
    );
    run_git(
        &checkout,
        [
            "remote",
            "add",
            "scope",
            &format!("{}/git/{view}/owner/repo", server.api_url),
        ],
    );
    checkout
}
