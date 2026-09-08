mod support;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    net::TcpListener,
    process::Command,
    sync::{Arc, Mutex},
    thread,
};
use support::*;
use tokio::sync::oneshot;

const OID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn expired_upload_operations_rotate_without_manual_journal_edits() {
    let cwd = TempDir::new("expired-upload-cwd");
    let file = cwd.path().join("photo.png");
    fs::write(&file, b"photo").unwrap();
    let server = MediaFixture::start(false);
    server.state.lock().unwrap().fail_expired_upload_once = true;
    let output = server
        .command(cwd.path())
        .args([
            "--json",
            "--repo",
            "owner/repo",
            "request",
            "edit",
            "--request",
            "req_one",
            "--attach",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let state = server.state.lock().unwrap();
    assert_eq!(state.prepare_operation_ids.len(), 2);
    assert_ne!(
        state.prepare_operation_ids[0],
        state.prepare_operation_ids[1]
    );
    assert_eq!(state.edits.len(), 1);
}

#[test]
fn attachment_only_edit_resumes_parts_and_preserves_machine_output() {
    let cwd = TempDir::new("attachment-resume-cwd");
    let file = cwd.path().join("walkthrough.mp4");
    let mut bytes = vec![7_u8; 8 * 1024 * 1024];
    bytes.extend_from_slice(b"trailer");
    fs::write(&file, bytes).unwrap();
    let server = MediaFixture::start(true);

    let first = server
        .command(cwd.path())
        .args([
            "--json",
            "--repo",
            "owner/repo",
            "request",
            "edit",
            "--request",
            "req_one",
            "--attach",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(6), "{first:?}");
    assert!(first.stdout.is_empty());

    let second = server
        .command(cwd.path())
        .args([
            "--json",
            "--repo",
            "owner/repo",
            "request",
            "edit",
            "--request",
            "req_one",
            "--attach",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let output: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(output["command"], "request.edit");
    assert_eq!(output["result"]["attachments"][0]["id"], "att_one");
    assert_eq!(output["result"]["attachments"][0]["state"], "Processing");
    let progress = String::from_utf8(second.stderr).unwrap();
    assert!(progress.contains("100%"), "{progress}");

    let state = server.state.lock().unwrap();
    assert_eq!(state.prepare_operation_ids.len(), 2);
    assert_eq!(
        state.prepare_operation_ids[0], state.prepare_operation_ids[1],
        "process retry must reuse the upload operation"
    );
    assert_eq!(
        state.part_attempts,
        vec![1, 2, 2],
        "the acknowledged first 8 MiB part must not be retransmitted"
    );
    assert_eq!(state.part_sizes[&1], 8 * 1024 * 1024);
    assert_eq!(state.part_sizes[&2], 7);
    assert_eq!(state.finish_parts.as_ref().unwrap().len(), 2);
    assert_eq!(
        state.edits[0]["description_markdown"],
        "[walkthrough.mp4](/request-attachments/att_one)"
    );
    assert_eq!(state.edits[0]["expected_description_markdown"], "");
    drop(state);
    server.finish();
}

#[test]
fn attachment_only_discussion_retry_reuses_the_client_mutation_id() {
    let cwd = TempDir::new("attachment-discussion-cwd");
    let file = cwd.path().join("walkthrough.mp4");
    fs::write(&file, b"video bytes").unwrap();
    let server = MediaFixture::start_with_failures(false, true);
    let args = [
        "--json",
        "--repo",
        "owner/repo",
        "request",
        "discussion",
        "start",
        "--request",
        "req_one",
        "--attach",
        file.to_str().unwrap(),
    ];

    let first = server.command(cwd.path()).args(args).output().unwrap();
    assert_eq!(first.status.code(), Some(6), "{first:?}");
    assert!(first.stdout.is_empty());

    let second = server.command(cwd.path()).args(args).output().unwrap();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let output: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(output["command"], "request.discussion.start");
    assert_eq!(output["result"]["attachments"][0]["id"], "att_one");

    let third = server.command(cwd.path()).args(args).output().unwrap();
    assert!(third.status.success(), "{third:?}");

    let state = server.state.lock().unwrap();
    assert_eq!(state.discussions.len(), 3);
    assert_eq!(
        state.discussions[0]["client_discussion_id"],
        state.discussions[1]["client_discussion_id"]
    );
    assert_ne!(
        state.discussions[1]["client_discussion_id"], state.discussions[2]["client_discussion_id"],
        "a later successful command must start a new post"
    );
    assert_eq!(
        state.prepare_operation_ids[0], state.prepare_operation_ids[1],
        "the failed post retry must reuse its attachment"
    );
    assert_ne!(
        state.prepare_operation_ids[1], state.prepare_operation_ids[2],
        "a later successful command must start a new attachment"
    );
    assert_eq!(
        state.discussions[1]["body_markdown"],
        "[walkthrough.mp4](/request-attachments/att_one)"
    );
    assert_eq!(state.prepare_targets[0]["kind"], "Discussion");
    assert!(state.prepare_targets[0]["discussion_id"].is_null());
    drop(state);
    server.finish();
}

#[test]
fn wait_failure_keeps_the_saved_post_and_upload_receipts_for_retry() {
    let cwd = TempDir::new("attachment-discussion-wait-retry-cwd");
    let file = cwd.path().join("walkthrough.mp4");
    fs::write(&file, b"video bytes").unwrap();
    let server = MediaFixture::start(false);
    server.state.lock().unwrap().fail_get_once = true;
    let args = [
        "--json",
        "--repo",
        "owner/repo",
        "request",
        "discussion",
        "start",
        "--request",
        "req_one",
        "--attach",
        file.to_str().unwrap(),
        "--wait",
    ];

    let first = server.command(cwd.path()).args(args).output().unwrap();
    assert_eq!(first.status.code(), Some(6), "{first:?}");
    assert!(first.stdout.is_empty());
    let failure: Value = serde_json::from_str(
        String::from_utf8_lossy(&first.stderr)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(failure["retryable"], true);
    assert_eq!(failure["recovery"]["saved"], true);
    assert_eq!(failure["recovery"]["request_id"], "req_one");
    assert_eq!(failure["recovery"]["discussion"]["id"], "dsc_one");
    assert_eq!(failure["recovery"]["attachments"][0]["id"], "att_one");

    let second = server.command(cwd.path()).args(args).output().unwrap();
    assert!(second.status.success(), "{second:?}");
    let success: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(success["result"]["attachments"][0]["state"], "Ready");

    let state = server.state.lock().unwrap();
    assert_eq!(state.discussions.len(), 2);
    assert_eq!(
        state.discussions[0]["client_discussion_id"],
        state.discussions[1]["client_discussion_id"]
    );
    assert_eq!(
        state.prepare_operation_ids[0], state.prepare_operation_ids[1],
        "the saved post retry must reuse its attachment"
    );
    drop(state);
    server.finish();
}

#[test]
fn expired_media_grant_is_renewed_and_receipts_are_reconciled() {
    let cwd = TempDir::new("attachment-grant-cwd");
    let file = cwd.path().join("walkthrough.mp4");
    let mut bytes = vec![3_u8; 8 * 1024 * 1024];
    bytes.extend_from_slice(b"renewed");
    fs::write(&file, bytes).unwrap();
    let server = MediaFixture::start(false);
    server.state.lock().unwrap().fail_grant_once = true;

    let output = server
        .command(cwd.path())
        .args([
            "--json",
            "--repo",
            "owner/repo",
            "request",
            "edit",
            "--request",
            "req_one",
            "--attach",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Renewed media transfer grant")
    );
    let state = server.state.lock().unwrap();
    assert_eq!(state.prepare_operation_ids.len(), 2);
    assert_eq!(
        state.prepare_operation_ids[0],
        state.prepare_operation_ids[1]
    );
    assert_eq!(state.part_attempts, vec![1, 2, 2]);
    drop(state);
    server.finish();
}

#[test]
fn lost_edit_response_reuses_finished_upload_without_duplicate_reference() {
    let cwd = TempDir::new("attachment-edit-response-cwd");
    let file = cwd.path().join("walkthrough.mp4");
    fs::write(&file, b"short clip").unwrap();
    let server = MediaFixture::start(false);
    server.state.lock().unwrap().fail_edit_once = true;
    let args = [
        "--json",
        "--repo",
        "owner/repo",
        "request",
        "edit",
        "--request",
        "req_one",
        "--attach",
        file.to_str().unwrap(),
    ];

    let first = server.command(cwd.path()).args(args).output().unwrap();
    assert_eq!(first.status.code(), Some(6), "{first:?}");
    let second = server.command(cwd.path()).args(args).output().unwrap();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );

    let state = server.state.lock().unwrap();
    assert_eq!(state.prepare_operation_ids.len(), 2);
    assert_eq!(
        state.prepare_operation_ids[0],
        state.prepare_operation_ids[1]
    );
    assert_eq!(state.part_attempts, vec![1]);
    assert_eq!(state.edits.len(), 2);
    let reference = "[walkthrough.mp4](/request-attachments/att_one)";
    assert_eq!(state.edits[0]["description_markdown"], reference);
    assert_eq!(state.edits[0]["expected_description_markdown"], "");
    assert_eq!(state.edits[1]["description_markdown"], reference);
    assert_eq!(state.edits[1]["expected_description_markdown"], reference);
    assert_eq!(state.current_description.matches(reference).count(), 1);
    drop(state);
    server.finish();
}

#[test]
fn changed_file_content_starts_a_new_upload_operation() {
    let cwd = TempDir::new("attachment-changed-cwd");
    let file = cwd.path().join("walkthrough.mp4");
    fs::write(&file, b"first clip").unwrap();
    let server = MediaFixture::start(false);
    let args = [
        "--json",
        "--repo",
        "owner/repo",
        "request",
        "edit",
        "--request",
        "req_one",
        "--attach",
        file.to_str().unwrap(),
    ];
    let first = server.command(cwd.path()).args(args).output().unwrap();
    assert!(first.status.success(), "{first:?}");

    fs::write(&file, b"second clip with changed bytes").unwrap();
    {
        let mut state = server.state.lock().unwrap();
        state.finished = false;
        state.acknowledged.clear();
    }
    let second = server.command(cwd.path()).args(args).output().unwrap();
    assert!(second.status.success(), "{second:?}");

    let state = server.state.lock().unwrap();
    assert_eq!(state.prepare_operation_ids.len(), 2);
    assert_ne!(
        state.prepare_operation_ids[0],
        state.prepare_operation_ids[1]
    );
    drop(state);
    server.finish();
}

#[test]
fn wait_polls_processing_attachment_to_a_terminal_state() {
    let cwd = TempDir::new("attachment-wait-cwd");
    let file = cwd.path().join("walkthrough.mp4");
    fs::write(&file, b"clip").unwrap();
    let server = MediaFixture::start(false);
    let output = server
        .command(cwd.path())
        .args([
            "--json",
            "--repo",
            "owner/repo",
            "request",
            "edit",
            "--request",
            "req_one",
            "--attach",
            file.to_str().unwrap(),
            "--wait",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["result"]["attachments"][0]["state"], "Ready");
    assert_eq!(server.state.lock().unwrap().get_attachment_calls, 1);
    server.finish();
}

#[test]
fn attachment_only_reply_and_reopen_use_the_reply_target() {
    for command in ["reply", "reopen"] {
        let cwd = TempDir::new(&format!("attachment-{command}-cwd"));
        let file = cwd.path().join("walkthrough.mp4");
        fs::write(&file, b"clip").unwrap();
        let server = MediaFixture::start(false);
        let output = server
            .command(cwd.path())
            .args([
                "--json",
                "--repo",
                "owner/repo",
                "request",
                "discussion",
                command,
                "dsc_one",
                "--request",
                "req_one",
                "--attach",
                file.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{command}: {output:?}");
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["command"], format!("request.discussion.{command}"));
        assert_eq!(value["result"]["attachments"][0]["id"], "att_one");
        let state = server.state.lock().unwrap();
        assert_eq!(state.prepare_targets[0]["kind"], "Reply");
        assert_eq!(state.prepare_targets[0]["discussion_id"], "dsc_one");
        assert_eq!(state.replies[0].0, command);
        assert_eq!(
            state.replies[0].1["body_markdown"],
            "[walkthrough.mp4](/request-attachments/att_one)"
        );
        drop(state);
        server.finish();
    }
}

#[test]
fn request_edit_appends_attachment_to_supplied_description_file() {
    let cwd = TempDir::new("attachment-description-file-cwd");
    let media = cwd.path().join("walkthrough.mp4");
    let description = cwd.path().join("description.md");
    fs::write(&media, b"clip").unwrap();
    fs::write(&description, "Details from a file\n").unwrap();
    let server = MediaFixture::start(false);
    let output = server
        .command(cwd.path())
        .args([
            "--json",
            "--repo",
            "owner/repo",
            "request",
            "edit",
            "--request",
            "req_one",
            "--description-file",
            description.to_str().unwrap(),
            "--attach",
            media.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        server.state.lock().unwrap().edits[0]["description_markdown"],
        "Details from a file\n\n[walkthrough.mp4](/request-attachments/att_one)"
    );
    server.finish();
}

#[derive(Default)]
struct FixtureState {
    api_url: String,
    fail_second_part_once: bool,
    fail_discussion_once: bool,
    fail_grant_once: bool,
    fail_expired_upload_once: bool,
    fail_edit_once: bool,
    fail_get_once: bool,
    finished: bool,
    current_description: String,
    prepare_operation_ids: Vec<String>,
    prepare_targets: Vec<Value>,
    last_prepare: Value,
    acknowledged: BTreeMap<u32, Value>,
    part_attempts: Vec<u32>,
    part_sizes: BTreeMap<u32, usize>,
    finish_parts: Option<Vec<Value>>,
    edits: Vec<Value>,
    discussions: Vec<Value>,
    get_attachment_calls: usize,
    replies: Vec<(String, Value)>,
}

struct MediaFixture {
    api_url: String,
    config: TempDir,
    state: Arc<Mutex<FixtureState>>,
    stop: oneshot::Sender<()>,
    thread: thread::JoinHandle<()>,
}

impl MediaFixture {
    fn start(fail_second_part_once: bool) -> Self {
        Self::start_with_failures(fail_second_part_once, false)
    }

    fn start_with_failures(fail_second_part_once: bool, fail_discussion_once: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(FixtureState {
            api_url: api_url.clone(),
            fail_second_part_once,
            fail_discussion_once,
            ..FixtureState::default()
        }));
        let config = TempDir::new("attachment-resume-config");
        install_session(config.path(), &api_url);
        let app_state = state.clone();
        let (stop, stopped) = oneshot::channel();
        let thread = thread::spawn(move || {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async move {
                    let app = Router::new()
                        .route("/v1/session", get(session))
                        .route("/v1/repos/owner/repo", get(repository))
                        .route(
                            "/v1/repos/owner/repo/requests/req_one",
                            get(request_detail).patch(edit_request),
                        )
                        .route(
                            "/v1/repos/owner/repo/requests/req_one/attachments/limits",
                            get(attachment_limits),
                        )
                        .route(
                            "/v1/repos/owner/repo/requests/req_one/attachments/prepare",
                            post(prepare_attachment),
                        )
                        .route(
                            "/v1/repos/owner/repo/requests/req_one/attachments/att_one/finish",
                            post(finish_attachment),
                        )
                        .route(
                            "/v1/repos/owner/repo/requests/req_one/attachments/att_one",
                            get(get_attachment),
                        )
                    .route(
                        "/v1/repos/owner/repo/requests/req_one/timeline",
                        post(create_discussion),
                    )
                    .route(
                        "/v1/repos/owner/repo/requests/req_one/threads/dsc_one/replies",
                        post(create_reply),
                    )
                    .route(
                        "/v1/repos/owner/repo/requests/req_one/threads/dsc_one/reopen-and-reply",
                        post(reopen_and_reply),
                    )
                        .route(
                            "/media/v1/uploads/upload_one/parts/{part_number}",
                            put(upload_part),
                        )
                        .layer(DefaultBodyLimit::max(9 * 1024 * 1024))
                        .with_state(app_state);
                    axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app)
                        .with_graceful_shutdown(async {
                            let _ = stopped.await;
                        })
                        .await
                        .unwrap();
                });
        });
        Self {
            api_url,
            config,
            state,
            stop,
            thread,
        }
    }

    fn command(&self, cwd: &std::path::Path) -> Command {
        let mut command = scope_command(cwd);
        command
            .env("SCOPE_API_URL", &self.api_url)
            .env("XDG_CONFIG_HOME", self.config.path());
        command
    }

    fn finish(self) {
        let _ = self.stop.send(());
        self.thread.join().unwrap();
    }
}

fn install_session(config: &std::path::Path, api_url: &str) {
    let key = api_url
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    fs::create_dir_all(config.join("scope/sessions")).unwrap();
    fs::write(
        config.join(format!("scope/sessions/cli-session-{key}")),
        "test-token",
    )
    .unwrap();
}

async fn session() -> Json<Value> {
    Json(
        json!({"identity":null,"user":{"id":"usr_test","handle":"owner","email":"test@example.test","email_verified":true}}),
    )
}

async fn repository() -> Json<Value> {
    Json(json!({
        "id":"repo_one","owner_handle":"owner","name":"repo",
        "git_remote_url":"https://scope.example/git/public/owner/repo",
        "lifecycle_state":"Ready","change_version":1,
        "access":{"actor":"Owner","can_read_private_files":true,"can_push":true,"can_change_file_visibility":true,"can_apply_changes":true,"can_manage_members":true,"can_delete_repo":true},
        "open_request_count":1,"request_permissions":{"can_start_request":true}
    }))
}

async fn request_detail(State(state): State<Arc<Mutex<FixtureState>>>) -> Json<Value> {
    let description = state.lock().unwrap().current_description.clone();
    Json(json!({"request":request_json(&description)}))
}

async fn attachment_limits() -> Json<Value> {
    Json(json!({
        "max_photo_bytes":26214400_u64,"max_video_bytes":524288000_u64,
        "max_video_duration_seconds":600,"max_attachments_per_content":10,
        "max_request_source_bytes":2147483648_u64,"max_repository_storage_bytes":21474836480_u64,
        "max_photo_pixels":50000000_u64,"preferred_part_bytes":8388608_u64,
        "max_concurrent_parts":2,
        "accepted_photo_media_types":["image/png","image/jpeg","image/webp","image/gif","image/heic","image/heif"],
        "accepted_video_media_types":["video/mp4","video/quicktime","video/webm"],
        "incomplete_upload_ttl_seconds":86400,"unbound_attachment_ttl_seconds":604800
    }))
}

async fn prepare_attachment(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Json(body): Json<Value>,
) -> Response {
    let mut state = state.lock().unwrap();
    state
        .prepare_operation_ids
        .push(body["operation_id"].as_str().unwrap().to_string());
    if state.fail_expired_upload_once {
        state.fail_expired_upload_once = false;
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "code":"attachment_upload_expired", "message":"upload expired", "retryable":false,
            })),
        )
            .into_response();
    }
    state.prepare_targets.push(body["target"].clone());
    state.last_prepare = body.clone();
    let acknowledged = state.acknowledged.values().cloned().collect::<Vec<_>>();
    let attachment_state = if state.finished {
        "Processing"
    } else {
        "Prepared"
    };
    Json(json!({
        "attachment":attachment_json(&body, attachment_state),
        "transfer":{
            "upload_id":"upload_one","media_base_url":format!("{}/media", state.api_url),
            "grant":"upload-grant","expires_at_unix":4102444800_u64,
            "preferred_part_bytes":8388608_u64,"max_concurrent_parts":2,
            "acknowledged_parts":acknowledged
        }
    }))
    .into_response()
}

async fn upload_part(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Path(part_number): Path<u32>,
    bytes: Bytes,
) -> Response {
    let mut state = state.lock().unwrap();
    state.part_attempts.push(part_number);
    if part_number == 2 && state.fail_grant_once {
        state.fail_grant_once = false;
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "code":"unauthorized","message":"upload grant expired","retryable":false
            })),
        )
            .into_response();
    }
    if part_number == 2 && state.fail_second_part_once {
        state.fail_second_part_once = false;
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "code":"ServiceUnavailable","message":"retry upload","retryable":true,
                "fields":{"paths":[]},"instruction":null,"error_reference":null
            })),
        )
            .into_response();
    }
    let receipt = json!({
        "part_number":part_number,"size_bytes":bytes.len(),
        "sha256":hex::encode(Sha256::digest(&bytes))
    });
    state.part_sizes.insert(part_number, bytes.len());
    state.acknowledged.insert(part_number, receipt.clone());
    Json(receipt).into_response()
}

async fn finish_attachment(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let mut state = state.lock().unwrap();
    state.finish_parts = Some(body["parts"].as_array().unwrap().clone());
    state.finished = true;
    Json(attachment_json(&state.last_prepare, "Processing"))
}

async fn get_attachment(State(state): State<Arc<Mutex<FixtureState>>>) -> Response {
    let mut state = state.lock().unwrap();
    state.get_attachment_calls += 1;
    if state.fail_get_once {
        state.fail_get_once = false;
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "code":"ServiceUnavailable","message":"status temporarily unavailable",
                "retryable":true,"fields":{"paths":[]},"instruction":null,
                "error_reference":null
            })),
        )
            .into_response();
    }
    Json(attachment_json(&state.last_prepare, "Ready")).into_response()
}

async fn edit_request(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Json(body): Json<Value>,
) -> Response {
    let mut state = state.lock().unwrap();
    state.edits.push(body.clone());
    state.current_description = body["description_markdown"].as_str().unwrap().to_string();
    if state.fail_edit_once {
        state.fail_edit_once = false;
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "code":"service_unavailable","message":"edit response lost","retryable":true
            })),
        )
            .into_response();
    }
    Json(json!({"request":request_json(&state.current_description)})).into_response()
}

async fn create_discussion(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Json(body): Json<Value>,
) -> Response {
    let mut state = state.lock().unwrap();
    state.discussions.push(body.clone());
    if state.fail_discussion_once {
        state.fail_discussion_once = false;
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "code":"ServiceUnavailable","message":"response lost","retryable":true,
                "fields":{"paths":[]},"instruction":null,"error_reference":null
            })),
        )
            .into_response();
    }
    Json(json!({
        "discussion":{
            "id":"dsc_one","request_id":"req_one",
            "client_discussion_id":body["client_discussion_id"],
            "opened_position":1,"last_activity_position":1,
            "author":{"id":"usr_test","handle":"owner"},
            "body_markdown":body["body_markdown"],"anchor":null,"status":"Open",
            "reply_count":0,"read_through_position":1,"unread_count":0,"latest_replies":[],
            "created_at_unix":1,"resolved_at_unix":null,"resolved_by":null
        }
    }))
    .into_response()
}

async fn create_reply(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    reply_response(state, "reply", body)
}

async fn reopen_and_reply(
    State(state): State<Arc<Mutex<FixtureState>>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    reply_response(state, "reopen", body)
}

fn reply_response(state: Arc<Mutex<FixtureState>>, operation: &str, body: Value) -> Json<Value> {
    state
        .lock()
        .unwrap()
        .replies
        .push((operation.to_string(), body.clone()));
    let reply = json!({
        "id":"rpl_one","discussion_id":"dsc_one","position":2,
        "author":{"id":"usr_test","handle":"owner"},
        "body_markdown":body["body_markdown"],"reply_to":null,"created_at_unix":2
    });
    Json(json!({
        "discussion":{
            "id":"dsc_one","request_id":"req_one","client_discussion_id":"original",
            "opened_position":1,"last_activity_position":2,
            "author":{"id":"usr_test","handle":"owner"},"body_markdown":"Original",
            "anchor":null,"status":"Open","reply_count":1,"read_through_position":2,
            "unread_count":0,"latest_replies":[reply.clone()],"created_at_unix":1,
            "resolved_at_unix":null,"resolved_by":null
        },
        "reply":reply
    }))
}

fn attachment_json(prepare: &Value, state: &str) -> Value {
    let media_type = prepare["declared_media_type"].as_str().unwrap();
    json!({
        "id":"att_one","request_id":"req_one","uploader_user_id":"usr_test",
        "filename":prepare["filename"],"declared_media_type":media_type,
        "detected_media_type":null,"kind":if media_type.starts_with("image/") { "Photo" } else { "Video" },
        "size_bytes":prepare["size_bytes"],"sha256":prepare["sha256"],
        "state":state,"original_download_available":state == "Ready",
        "failure":null,"image":null,"video":null,
        "derivatives":[],"created_at_unix":1,"updated_at_unix":1
    })
}

fn request_json(description: &str) -> Value {
    json!({
        "id":"req_one","name":"fix-one","title":"Fix one","description_markdown":description,
        "author_user_id":"usr_test","author_role":"Owner","audience":"Private",
        "base_main_oid":OID,"head_oid":OID,"state":"Draft","activity_version":0,
        "submitted_at_unix":null,"closed_at_unix":null,"closed_by_user_id":null,
        "merged_at_unix":null,"merged_by_user_id":null,"merged_head_oid":null,"merged_main_oid":null,
        "created_at_unix":1,"updated_at_unix":2,"invitees":[],
        "permissions":{"can_view_activity":true,"can_open_discussion":true,"can_reply_to_discussion":true,
            "can_edit_identity":true,"can_pull_branch":true,"can_push_branch":true,"can_submit":true,
            "can_manage_invitees":true,"can_leave_request":false,"can_close":true,"can_merge":true},
        "mergeability":{"status":"Draft","current_main_oid":OID,"request_head_oid":OID,"reason":null}
    })
}
