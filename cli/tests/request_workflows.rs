#[cfg(unix)]
#[path = "request_workflows/pull.rs"]
mod pull;
mod support;

use axum::{
    Json, Router,
    extract::{OriginalUri, Query},
    routing::get,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};
use support::*;

#[test]
fn request_reads_work_outside_a_checkout_with_explicit_repository() {
    let dir = TempDir::new("request-outside");
    let server = FixtureServer::start();
    let output = server
        .request_command(dir.path())
        .args([
            "list",
            "--state",
            "open",
            "--audience",
            "public",
            "--search",
            "fix",
            "--limit",
            "1",
        ])
        .output()
        .unwrap();
    let value = success(output);
    assert_eq!(value["command"], "request.list");
    assert_eq!(value["result"]["requests"].as_array().unwrap().len(), 1);
    assert_eq!(value["result"]["requests"][0]["name"], "fix-one");
    let output = server
        .request_command(dir.path())
        .args(["show", "--request", "req_one"])
        .output()
        .unwrap();
    assert_eq!(success(output)["result"]["request"]["id"], "req_one");
}

#[test]
fn request_diff_uses_server_revision_and_path_with_no_local_private_data() {
    let dir = TempDir::new("request-diff");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("private.txt"), "LOCAL PRIVATE SENTINEL").unwrap();
    let server = FixtureServer::start();
    let output = server
        .request_command(dir.path())
        .args([
            "diff",
            "--request",
            "req_one",
            "--revision",
            "rev_old",
            "--commit",
            OID,
            "--path",
            "space name.txt",
        ])
        .output()
        .unwrap();
    let value = success(output);
    assert_eq!(
        value["result"]["revisions"]["review_revision_id"],
        "rev_old"
    );
    assert_eq!(
        value["result"]["files"][0]["diff"]["new_content"]["text"],
        "server-visible\n"
    );
    assert!(!value.to_string().contains("LOCAL PRIVATE SENTINEL"));
    let seen = server.seen.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|uri| uri.contains("revision=rev_old") && uri.contains(OID)),
        "{seen:?}"
    );
    assert!(
        seen.iter()
            .any(|uri| uri.contains("/changes/rev_old/commits/")
                && uri.contains("path=space+name.txt")),
        "{seen:?}"
    );
}

#[test]
fn request_diff_rejects_a_commit_absent_from_visible_revision_inspection() {
    let dir = TempDir::new("request-absent-commit");
    let server = FixtureServer::start();
    let output = server
        .request_command(dir.path())
        .args([
            "diff",
            "--request",
            "req_one",
            "--revision",
            "rev_old",
            "--commit",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "not_found");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("visible revision inspection")
    );
    assert!(
        !server
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|uri| uri.contains("file-diff"))
    );
}

#[test]
fn request_diff_defaults_to_visible_text_changes() {
    let dir = TempDir::new("request-default-diff");
    let server = FixtureServer::start();
    let output = server
        .command(dir.path())
        .args([
            "--repo",
            "owner/repo",
            "request",
            "diff",
            "--request",
            "req_one",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("-old\n+server-visible"), "{stdout}");
}

#[test]
fn contributor_request_checks_use_request_permissions_without_maintainer_endpoints() {
    let dir = TempDir::new("request-contributor-checks");
    let server = FixtureServer::start();
    // Public contributors must not call the maintainer-only run history endpoint.
    let output = server
        .request_command(dir.path())
        .args(["checks", "--request", "req_one"])
        .output()
        .unwrap();
    let result = success(output);
    assert_eq!(result["result"]["head_oid"], OID);
    assert_eq!(result["result"]["mergeability"]["status"], "Draft");
    assert_eq!(result["result"]["workflow_runs_available"], false);
    assert_eq!(result["result"]["runs"], json!([]));
    assert!(server.seen.lock().unwrap().is_empty());
}

#[test]
fn maintainer_request_checks_filter_exact_head_across_run_history_pages() {
    let dir = TempDir::new("request-maintainer-checks");
    let mut repo = repository();
    repo["access"]["actor"] = "Member".into();
    let server = FixtureServer::with_repository(request(), repo);
    let output = server
        .request_command(dir.path())
        .args(["checks", "--request", "req_one"])
        .output()
        .unwrap();
    let result = success(output);
    assert_eq!(result["result"]["workflow_runs_available"], true);
    assert_eq!(result["result"]["runs"].as_array().unwrap().len(), 1);
    assert_eq!(result["result"]["runs"][0]["id"], "run_current");
    assert!(
        server
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|uri| uri.contains("after=next-page"))
    );
}

#[test]
fn dirty_request_checkout_fails_before_authentication_or_api_calls() {
    let dir = TempDir::new("request-checkout-dirty");
    create_repo_with_head(dir.path());
    fs::write(dir.path().join("README.md"), "keep my changes\n").unwrap();
    let output = scope_command(dir.path())
        .args(["--json", "request", "checkout", "--request", "req_one"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("commit or stash local changes"),
        "{combined}"
    );
    assert!(!combined.contains("scope login"), "{combined}");
    assert_eq!(
        fs::read_to_string(dir.path().join("README.md")).unwrap(),
        "keep my changes\n"
    );
}

#[test]
fn request_edit_reads_description_stdin_outside_checkout() {
    use std::io::Write;
    let dir = TempDir::new("request-description");
    let server = FixtureServer::start();
    let mut child = server
        .request_command(dir.path())
        .args(["edit", "--request", "req_one", "--description-file", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"first\n\nsecond\\n\n")
        .unwrap();
    success(child.wait_with_output().unwrap());
    assert_eq!(
        server.edited.lock().unwrap()["description_markdown"],
        "first\n\nsecond\\n\n"
    );
}

#[cfg(unix)]
#[test]
fn request_start_metadata_failure_can_retry_push_without_creating_another_request() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("request-start-recovery");
    create_repo_with_head(dir.path());
    let head = git_stdout(dir.path(), ["rev-parse", "HEAD"]);
    let bare = TempDir::new("request-recovery-bare");
    run_git(bare.path(), ["init", "--bare"]);
    run_git(dir.path(), ["push", bare.path().to_str().unwrap(), "main"]);
    let mut detail = request();
    detail["base_main_oid"] = head.clone().into();
    detail["head_oid"] = head.clone().into();
    detail["mergeability"]["current_main_oid"] = head.clone().into();
    detail["mergeability"]["request_head_oid"] = head.clone().into();
    let server = FixtureServer::with_request(detail);
    let public = format!("{}/git/public/owner/repo", server.server.api_url);
    let permissioned = format!("{}/git/permissioned/owner/repo", server.server.api_url);
    let file_url = reqwest::Url::from_directory_path(bare.path())
        .unwrap()
        .to_string();

    run_git(dir.path(), ["remote", "add", "scope", &public]);
    run_git(
        dir.path(),
        ["remote", "set-url", "--push", "scope", &permissioned],
    );

    let shim = TempDir::new("request-git-transport");
    let shim_path = shim.path().join("git");
    fs::write(
        &shim_path,
        r#"#!/bin/bash
args=()
for arg in "$@"; do
  if [[ "$arg" == "$SCOPE_TEST_PUBLIC_URL" || "$arg" == "$SCOPE_TEST_PERMISSIONED_URL" ]]; then
    args+=("$SCOPE_TEST_FILE_URL")
  else
    args+=("$arg")
  fi
done
exec "$SCOPE_TEST_REAL_GIT" "${args[@]}"
"#,
    )
    .unwrap();
    fs::set_permissions(&shim_path, fs::Permissions::from_mode(0o700)).unwrap();
    let existing_path = std::env::var_os("PATH").unwrap();
    let real_git = std::env::split_paths(&existing_path)
        .map(|dir| dir.join("git"))
        .find(|path| path.is_file())
        .unwrap();
    let test_path = std::env::join_paths(
        std::iter::once(shim.path().to_path_buf()).chain(std::env::split_paths(&existing_path)),
    )
    .unwrap();
    let command = || {
        let mut command = server.command(dir.path());
        command
            .env("PATH", &test_path)
            .env("SCOPE_TEST_REAL_GIT", &real_git)
            .env("SCOPE_TEST_PUBLIC_URL", &public)
            .env("SCOPE_TEST_PERMISSIONED_URL", &permissioned)
            .env("SCOPE_TEST_FILE_URL", &file_url);
        command
    };
    let hook = dir.path().join(".git/hooks/post-checkout");
    fs::write(&hook, "#!/bin/sh\n: > .git/config.lock\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    let output = command()
        .args(["--json", "request", "start", "fix-one"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    let error: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    assert_eq!(error["recovery"]["request_id"], "req_one");
    assert_eq!(error["recovery"]["failed_step"], "save_local_metadata");
    assert_eq!(error["recovery"]["remote_push_confirmed"], false);
    assert_eq!(
        git_stdout(dir.path(), ["branch", "--show-current"]),
        "fix-one"
    );
    assert_eq!(git_stdout(dir.path(), ["rev-parse", "HEAD"]), head);
    fs::remove_file(hook).unwrap();
    fs::remove_file(dir.path().join(".git/config.lock")).unwrap();
    let output = command()
        .args([
            "--json",
            "request",
            "push",
            "--remote",
            "scope",
            "--request",
            "req_one",
        ])
        .output()
        .unwrap();
    assert_eq!(success(output)["command"], "request.push");
    assert_eq!(
        git_stdout(bare.path(), ["rev-parse", "refs/heads/fix-one"]),
        head
    );
    assert_eq!(
        server
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.as_str() == "POST request")
            .count(),
        1
    );
}

const OID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn success(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

struct FixtureServer {
    server: TestServer,
    seen: Arc<Mutex<Vec<String>>>,
    edited: Arc<Mutex<Value>>,
}

impl FixtureServer {
    fn start() -> Self {
        Self::with_request(request())
    }

    fn with_request(detail: Value) -> Self {
        Self::with_repository(detail, repository())
    }

    fn with_repository(detail: Value, repo: Value) -> Self {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let inspected = seen.clone();
        let edited = Arc::new(Mutex::new(Value::Null));
        let captured = edited.clone();
        let revisions_seen = inspected.clone();
        let started = inspected.clone();
        let run_seen = inspected.clone();
        let start_detail = detail.clone();
        let show_detail = detail.clone();
        let app = Router::new()
                    .route("/v1/session", get(|| async { Json(json!({"identity": null, "user": {"id":"usr_test","handle":"owner","email":"test@example.test","email_verified":true}})) }))
                    .route("/v1/repos/owner/repo", get(move || { let repo=repo.clone(); async move { Json(repo) } }))
                    .route("/v1/repos/owner/repo/runs", get(move |OriginalUri(uri): OriginalUri, Query(query): Query<HashMap<String,String>>| { let seen=run_seen.clone(); async move { seen.lock().unwrap().push(uri.to_string()); if query.contains_key("after") { Json(json!({"runs":[run_summary("run_current", OID)], "next_cursor":null})) } else { Json(json!({"runs":[run_summary("run_stale", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")], "next_cursor":"next-page"})) } } }))
                    .route("/v1/repos/owner/repo/requests", get(|| async { Json(json!({"requests":[list_item("req_one", "fix-one", "Open"), list_item("req_two", "fix-two", "Closed"), list_item("req_three", "fix-three", "Open")], "next_cursor":null})) }).post(move || { let started=started.clone(); let detail=start_detail.clone(); async move { started.lock().unwrap().push("POST request".to_string()); Json(json!({"request":detail})) } }))
                    .route("/v1/repos/owner/repo/requests/req_one", get(move || { let detail=show_detail.clone(); async move { Json(json!({"request":detail})) } }).patch(move |Json(body): Json<Value>| { let captured=captured.clone(); async move { *captured.lock().unwrap()=body; Json(json!({"request":request()})) } }))
                    .route("/v1/repos/owner/repo/requests/req_one/changes", get(move |OriginalUri(uri): OriginalUri, Query(query): Query<HashMap<String,String>>| { let inspected=revisions_seen.clone(); async move { inspected.lock().unwrap().push(uri.to_string()); Json(json!({"review_revision_id":query.get("revision").map(String::as_str).unwrap_or("rev_old"), "revisions":[{"id":"rev_old","position":1,"actor":{"id":"usr_test","handle":"owner"},"old_head_oid":null,"new_head_oid":OID,"commits":[{"oid":OID,"parent_oids":[],"author":"owner","authored_at_unix":1,"message":"Old revision","change_count":1,"files":[{"path":"space name.txt","kind":"Modified","old_mode":"100644","new_mode":"100644","old_oid":OID,"new_oid":OID,"visibility":"Public"}],"files_truncated":false}],"inspection":"Complete","created_at_unix":1}],"has_earlier_revisions":false})) } }))
                    .route("/v1/repos/owner/repo/requests/req_one/changes/rev_old/commits/{commit}/file-diff", get(move |OriginalUri(uri): OriginalUri| { let inspected=inspected.clone(); async move { inspected.lock().unwrap().push(uri.to_string()); Json(json!({"path":"space name.txt","kind":"Modified","old_mode":"100644","new_mode":"100644","old_content":{"kind":"text","text":"old\n"},"new_content":{"kind":"text","text":"server-visible\n"}})) } }));
        Self {
            server: TestServer::new(app),
            seen,
            edited,
        }
    }
    fn request_command(&self, cwd: &std::path::Path) -> Command {
        let mut command = self.server.command(cwd);
        command.args(["--json", "--repo", "owner/repo", "request"]);
        command
    }
    fn command(&self, cwd: &std::path::Path) -> Command {
        self.server.command(cwd)
    }
}

fn repository() -> Value {
    json!({"id":"repo_one","owner_handle":"owner","name":"repo","git_remote_url":"https://scope.example/git/public/owner/repo","lifecycle_state":"Ready","change_version":1,"access":{"actor":"Public","can_read_private_files":false,"can_push":false,"can_change_file_visibility":false,"can_manage_members":false,"can_delete_repo":false},"open_request_count":2,"request_permissions":{"can_start_request":true}})
}
fn list_item(id: &str, name: &str, state: &str) -> Value {
    json!({"id":id,"name":name,"title":name,"author_role":"Public","audience":"Public","head_oid":OID,"state":state,"submitted_at_unix":1,"updated_at_unix":2,"mergeability":{"status":"Draft","current_main_oid":OID,"request_head_oid":OID,"reason":null}})
}
fn request() -> Value {
    json!({"id":"req_one","name":"fix-one","title":"Fix one","description_markdown":"","author_user_id":"usr_test","author_role":"Public","audience":"Public","base_main_oid":OID,"head_oid":OID,"state":"Draft","activity_version":0,"submitted_at_unix":null,"closed_at_unix":null,"closed_by_user_id":null,"merged_at_unix":null,"merged_by_user_id":null,"merged_head_oid":null,"merged_main_oid":null,"created_at_unix":1,"updated_at_unix":2,"invitees":[],"permissions":{"can_view_activity":false,"can_open_discussion":false,"can_reply_to_discussion":false,"can_edit_identity":true,"can_pull_branch":false,"can_push_branch":true,"can_submit":false,"can_manage_invitees":false,"can_leave_request":false,"can_close":false,"can_merge":false},"mergeability":{"status":"Draft","current_main_oid":OID,"request_head_oid":OID,"reason":null}})
}

fn run_summary(id: &str, oid: &str) -> Value {
    json!({"id":id,"workflow_name":"Checks","git_oid":oid,"trigger":"manual","state":"succeeded","cancellation_requested":false,"created_at_unix":1,"updated_at_unix":2,"completed_at_unix":2,"can_cancel":false,"can_retry":false})
}
