use super::*;
use scope_api_contract::routes::{repo_request_checks, repo_request_checks_approve};
use scope_domain::requests::{RequestCheck, RequestCheckEvaluation};
use scope_domain::runs::source::RunTrigger;
use scope_postgres::db::RecordRequestChecksCommand;

fn request_workflow() -> String {
    WORKFLOW.replacen("  manual: true", "  manual: true\n  request: true", 1)
}

fn checks_route(request_id: &str) -> String {
    repo_request_checks(TEST_REPO_OWNER, TEST_REPO_NAME, request_id)
}

fn approve_route(request_id: &str) -> String {
    repo_request_checks_approve(TEST_REPO_OWNER, TEST_REPO_NAME, request_id)
}

fn merge_route(request_id: &str) -> String {
    scope_api_contract::routes::repo_request_merge(TEST_REPO_OWNER, TEST_REPO_NAME, request_id)
}

async fn submit(app: axum::Router, request_id: &str, bearer: &str) {
    let submitted = api_request(
        app,
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}/submit"),
        Some(bearer),
        Some("{}"),
    )
    .await;
    assert_eq!(submitted.status(), StatusCode::OK);
}

/// An open private request the owner pushed, carrying the given workflow files
/// at its head. The server stays alive for the caller's later requests.
async fn owner_request_push(
    label: &str,
    workflows: &[(&str, String)],
) -> (AppState, String, TestServer) {
    let (state, source, _base_head) =
        super::super::push_intent_completion::published_git_fixture(label).await;
    let app = router(state.clone());
    let bearer = bearer_header();
    let started = api_request(
        app.clone(),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer),
        Some(r#"{"name":"checks","audience":"Private"}"#),
    )
    .await;
    let request_id = expect_json(started, StatusCode::OK).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (origin, server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer);
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    for (path, content) in workflows {
        fs::write(source.join(path), content).unwrap();
    }
    push_change(
        &source,
        &remote,
        "refs/heads/checks",
        "request.txt",
        "request work\n",
        "request change",
    )
    .unwrap();
    submit(app, &request_id, &bearer).await;
    (state, request_id, server)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_maintainers_push_starts_the_request_workflows_at_its_head() {
    let (state, request_id, _server) = owner_request_push(
        "request-checks-started",
        &[(".scope/runs/checks.yml", request_workflow())],
    )
    .await;

    let checks = expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &checks_route(&request_id),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;

    assert_eq!(checks["state"], "started");
    assert_eq!(checks["can_approve"], false);
    assert_eq!(checks["message"], serde_json::Value::Null);
    assert_eq!(checks["checks"].as_array().unwrap().len(), 1);
    assert_eq!(
        checks["checks"][0]["workflow_path"],
        "/.scope/runs/checks.yml"
    );
    assert_eq!(checks["checks"][0]["workflow_name"], "checks");
    assert_eq!(checks["checks"][0]["run_state"], "queued");
    assert_eq!(checks["mergeability"]["status"], "ChecksPending");
    assert_eq!(checks["mergeability"]["reason"], "checks have not finished");

    let run = state
        .metadata
        .runs()
        .run(checks["checks"][0]["run_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.trigger, RunTrigger::Request);
    assert_eq!(run.source.git_oid(), checks["head_oid"].as_str().unwrap());
    assert_eq!(run.requested_by_user_id.as_deref(), Some(&*test_owner_id()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_head_whose_workflows_never_ask_for_requests_owes_no_checks() {
    let (state, request_id, _server) = owner_request_push(
        "request-checks-none",
        &[(".scope/runs/manual.yml", WORKFLOW.to_string())],
    )
    .await;

    let checks = expect_json(
        api_request(
            router(state),
            "GET",
            &checks_route(&request_id),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;

    assert_eq!(checks["state"], "no-checks");
    assert_eq!(checks["can_approve"], false);
    assert!(checks["checks"].as_array().unwrap().is_empty());
    assert_eq!(checks["mergeability"]["status"], "Ready");
    assert_eq!(checks["mergeability"]["reason"], serde_json::Value::Null);
}

/// The evaluation a non-maintainer's push records for the request's current head.
/// Seeded here because `.scope/` is private, so nothing a public contributor can
/// see or push carries a workflow for their own head to ask for.
async fn record_awaiting_approval(state: &AppState, request_id: &str) {
    let request = stored_request(state, request_id).await;
    let workflow = request_workflow();
    let revisions = scope_run_config::parse_workflow_set(
        TEST_REPO_ID,
        [("/.scope/runs/checks.yml", workflow.as_bytes())],
    )
    .unwrap();
    let checks = revisions.iter().map(RequestCheck::for_revision).collect();
    let evaluation = RequestCheckEvaluation::awaiting_approval(
        &request.id,
        &request.head_oid,
        checks,
        unix_now(),
    )
    .unwrap();
    state
        .metadata
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            evaluation,
            revisions,
            runs: Vec::new(),
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checks_awaiting_approval_wait_for_a_maintainer_and_gate_the_merge() {
    let (state, _owner_source) = test_state_with_mergeable_request("request-checks-approval").await;
    insert_member_user(&state).await;
    let (_source, _remote, _server, _head) =
        request_checkout(&state, "request-checks-approval-contributor").await;
    record_awaiting_approval(&state, REQUEST_ID).await;
    let app = router(state.clone());
    let public = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    submit(app.clone(), REQUEST_ID, &public).await;

    let waiting = expect_json(
        api_request(
            app.clone(),
            "GET",
            &checks_route(REQUEST_ID),
            Some(&member),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(waiting["state"], "awaiting-approval");
    assert_eq!(waiting["can_approve"], true);
    assert_eq!(waiting["checks"].as_array().unwrap().len(), 1);
    assert_eq!(
        waiting["checks"][0]["workflow_path"],
        "/.scope/runs/checks.yml"
    );
    assert_eq!(waiting["checks"][0]["run_id"], serde_json::Value::Null);
    assert_eq!(waiting["checks"][0]["run_state"], serde_json::Value::Null);
    assert_eq!(waiting["mergeability"]["status"], "ChecksAwaitingApproval");

    let contributor_view = expect_json(
        api_request(
            app.clone(),
            "GET",
            &checks_route(REQUEST_ID),
            Some(&public),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(contributor_view["state"], "awaiting-approval");
    assert_eq!(contributor_view["can_approve"], false);

    let refused = api_request(
        app.clone(),
        "POST",
        &approve_route(REQUEST_ID),
        Some(&public),
        Some("{}"),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    let unapproved_merge = api_request(
        app.clone(),
        "POST",
        &merge_route(REQUEST_ID),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(
        expect_json(unapproved_merge, StatusCode::CONFLICT).await["message"],
        "checks are waiting for a maintainer to start them"
    );

    let approved = expect_json(
        api_request(
            app.clone(),
            "POST",
            &approve_route(REQUEST_ID),
            Some(&member),
            Some("{}"),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(approved["state"], "started");
    assert_eq!(approved["can_approve"], false);
    assert_eq!(approved["checks"][0]["run_state"], "queued");
    assert_eq!(approved["mergeability"]["status"], "ChecksPending");
    let run = state
        .metadata
        .runs()
        .run(approved["checks"][0]["run_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(run.trigger, RunTrigger::Request);
    assert_eq!(
        run.requested_by_user_id.as_deref(),
        Some(&*member_user_id())
    );

    let pending_merge =
        api_request(app, "POST", &merge_route(REQUEST_ID), Some(&member), None).await;
    assert_eq!(
        expect_json(pending_merge, StatusCode::CONFLICT).await["message"],
        "checks have not finished"
    );
}
