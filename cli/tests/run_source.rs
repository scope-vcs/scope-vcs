mod support;

use axum::{
    Json, Router,
    body::Bytes,
    extract::Query,
    http::HeaderMap,
    routing::{get, post},
};
use scope_api_contract::CreateManualRunQuery;
use std::{
    fs,
    sync::{Arc, Mutex},
};
use support::*;

#[test]
fn run_resolves_before_bundling_and_uploads_only_unknown_sources() {
    for known in [true, false] {
        let checkout = TempDir::new("run-source");
        create_repo_with_head(checkout.path());
        let oid = std::process::Command::new("git")
            .current_dir(checkout.path())
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let oid = String::from_utf8(oid.stdout).unwrap().trim().to_string();
        let upload = Arc::new(Mutex::new(None));
        let uploaded = upload.clone();
        let known_oid = oid.clone();
        let changing_checkout = checkout.path().to_path_buf();
        let uploaded_oid = oid.clone();
        let app = Router::new()
                    .route("/v1/session", get(|| async { Json(serde_json::json!({"identity":null,"user":{"id":"user-test","handle":"owner","email":"owner@example.test","email_verified":true}})) }))
                    .route("/v1/repos/owner/repo/runs/resolve", post(move |Query(query): Query<CreateManualRunQuery>, headers: HeaderMap, body: Bytes| {
                        let oid = known_oid.clone();
                        let checkout = changing_checkout.clone();
                        async move {
                            if !known {
                                fs::write(checkout.join("concurrent.txt"), "later commit").unwrap();
                                run_git(&checkout, ["add", "."]);
                                commit_all(&checkout, "concurrent commit");
                            }
                            assert_eq!(headers["authorization"], "Bearer test-token");
                            assert!(body.is_empty());
                            assert_eq!(query.git_oid, oid);
                            assert_eq!(query.workflow, "checks");
                            Json(if known { serde_json::json!({"status":"queued","run":run_response(&query)}) } else { serde_json::json!({"status":"upload-required"}) })
                        }
                    }))
                    .route("/v1/repos/owner/repo/runs", post(move |Query(query): Query<CreateManualRunQuery>, body: Bytes| {
                        let upload = uploaded.clone();
                        let oid = uploaded_oid.clone();
                        async move {
                            assert!(!known, "known source must not upload");
                            assert_eq!(query.git_oid, oid);
                            *upload.lock().unwrap() = Some(body.to_vec());
                            Json(run_response(&query))
                        }
                    }));
        let server = TestServer::new(app);
        run_git(
            checkout.path(),
            [
                "remote",
                "add",
                "scope",
                &format!("{}/git/permissioned/owner/repo", server.api_url),
            ],
        );
        let config = &server.config;
        let trace = config.path().join("git.trace");
        let output = server
            .command(checkout.path())
            .env("GIT_TRACE", &trace)
            .args(["--json", "run", "start", "checks", "--no-watch"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(receipt["command"], "run.start");
        let trace = fs::read_to_string(trace).unwrap();
        assert_eq!(trace.contains("git bundle create"), !known, "{trace}");
        let upload = upload.lock().unwrap().take();
        assert_eq!(upload.is_some(), !known);
        if let Some(bundle) = upload {
            let path = config.path().join("uploaded.bundle");
            fs::write(&path, bundle).unwrap();
            let heads = std::process::Command::new("git")
                .args(["bundle", "list-heads"])
                .arg(path)
                .output()
                .unwrap();
            assert!(heads.status.success());
            assert!(String::from_utf8_lossy(&heads.stdout).contains(&oid));
        }
        eprintln!(
            "known={known}: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        );
    }
}

fn run_response(query: &CreateManualRunQuery) -> serde_json::Value {
    serde_json::json!({
        "id":format!("run_{}", query.request_id), "repository_id":"owner/repo", "workflow_name":"checks", "git_oid":query.git_oid,
        "state":"queued", "cancellation_requested":false, "logs_truncated":false,
        "created_at_unix":1, "updated_at_unix":1, "completed_at_unix":null
    })
}
