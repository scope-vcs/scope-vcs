use super::*;
use scope_domain::requests::{
    RecordRequestRevisionInput, RequestActorRole, RequestAudience, RequestCheck,
    RequestCheckEvaluation, StartRequestInput,
};
use scope_domain::runs::run::RunState;
use scope_postgres::db::{RecordRequestChecksCommand, SubmitRequestCommand};

const AUTO_REQUEST_ID: &str = "req_auto_merge";
const AUTO_REQUEST_NAME: &str = "auto-merge";
const FIRST_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SECOND_HEAD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const THIRD_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";
const PUBLIC_AUTO_SUBJECT: &str = "user_auto_merge_public";
const PUBLIC_AUTO_EMAIL: &str = "auto-merge-public@example.com";

fn auto_merge_route(request_id: &str) -> String {
    scope_api_contract::routes::repo_request_auto_merge(TEST_REPO_OWNER, TEST_REPO_NAME, request_id)
}

async fn open_request_with_revision(
    request_id: &str,
    author_user_id: String,
    author_role: RequestActorRole,
    audience: RequestAudience,
) -> (AppState, String) {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    if author_user_id != test_owner_id() {
        state
            .metadata
            .auth()
            .insert_user_for_tests(test_user(
                &author_user_id,
                "auto-merge-public",
                PUBLIC_AUTO_EMAIL,
            ))
            .await
            .unwrap();
    }
    drain_outbox(&state, "request-auto-merge-test").await;
    state
        .metadata
        .requests()
        .start_request(StartRequestInput {
            id: request_id.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: request_id.replace('_', "-"),
            author_user_id: author_user_id.clone(),
            title: Some("Auto merge request".to_string()),
            author_role,
            audience,
            base_main_oid: FIRST_HEAD.to_string(),
            event_id: format!("event_{request_id}_started"),
            now_unix: 2,
        })
        .await
        .unwrap();
    let revision_id = format!("event_{request_id}_revision_1");
    record_revision(
        &state,
        request_id,
        &author_user_id,
        None,
        SECOND_HEAD,
        &revision_id,
        3,
    )
    .await;
    state
        .metadata
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: request_id.to_string(),
            actor_user_id: author_user_id,
            event_id: format!("event_{request_id}_submitted"),
            now_unix: 4,
        })
        .await
        .unwrap();
    (state, revision_id)
}

async fn record_revision(
    state: &AppState,
    request_id: &str,
    actor_user_id: &str,
    expected_old_head_oid: Option<&str>,
    new_head_oid: &str,
    event_id: &str,
    now_unix: u64,
) {
    let mut git_snapshot = scope_object_store::put_content_object(
        state.object_store.as_ref(),
        ContentObjectKind::GitBundle,
        format!("snapshot for {event_id}").into_bytes(),
    )
    .unwrap();
    git_snapshot.git_oid = new_head_oid.to_string();
    state
        .metadata
        .requests()
        .record_request_revision(
            RecordRequestRevisionInput {
                request_id: request_id.to_string(),
                actor_user_id: actor_user_id.to_string(),
                actor_can_edit: true,
                expected_old_head_oid: expected_old_head_oid.map(str::to_string),
                new_head_oid: new_head_oid.to_string(),
                git_snapshot,
                event_id: event_id.to_string(),
                body: None,
                now_unix,
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
}

async fn record_queued_request_check(state: &AppState, request_id: &str) -> String {
    let request = state
        .metadata
        .requests()
        .request_for_tests(request_id)
        .await
        .unwrap()
        .unwrap();
    let workflow = WORKFLOW.replacen("  manual: true", "  manual: true\n  request: true", 1);
    let revisions = scope_run_config::parse_workflow_set(
        TEST_REPO_ID,
        [("/.scope/runs/checks.yml", workflow.as_bytes())],
    )
    .unwrap();
    let mut checks = Vec::new();
    let mut runs = Vec::new();
    for revision in &revisions {
        let mut check = RequestCheck::for_revision(revision);
        let run = check.run(&request, revision, &test_owner_id(), 5).unwrap();
        check.run_id = Some(run.id.clone());
        checks.push(check);
        runs.push(run);
    }
    assert_eq!(runs.len(), 1);
    let run_id = runs[0].id.clone();
    state
        .metadata
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            evaluation: RequestCheckEvaluation::started(&request.id, &request.head_oid, checks, 5)
                .unwrap(),
            revisions,
            runs,
        })
        .await
        .unwrap();
    run_id
}

async fn auto_merge_json(
    app: axum::Router,
    method: &str,
    request_id: &str,
    bearer: &str,
    body: Option<&str>,
    expected: StatusCode,
) -> serde_json::Value {
    expect_json(
        api_request(
            app,
            method,
            &auto_merge_route(request_id),
            Some(bearer),
            body,
        )
        .await,
        expected,
    )
    .await
}

async fn authorize(
    app: axum::Router,
    request_id: &str,
    revision_id: &str,
    head_oid: &str,
    bearer: &str,
) -> serde_json::Value {
    auto_merge_json(
        app,
        "POST",
        request_id,
        bearer,
        Some(
            &serde_json::json!({
                "expected_revision_id": revision_id,
                "expected_head_oid": head_oid,
            })
            .to_string(),
        ),
        StatusCode::OK,
    )
    .await
}

async fn native_open_request(label: &str) -> (AppState, String, String, TestServer) {
    let (state, source, _main_head) =
        super::push_intent_completion::published_git_fixture(label).await;
    let app = router(state.clone());
    let bearer = bearer_header();
    let started = expect_json(
        api_request(
            app.clone(),
            "POST",
            &format!("/v1/repos/{TEST_REPO_ID}/requests"),
            Some(&bearer),
            Some(
                &serde_json::json!({
                    "name": AUTO_REQUEST_NAME,
                    "audience": "Private",
                })
                .to_string(),
            ),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let request_id = started["request"]["id"].as_str().unwrap().to_string();
    let (origin, server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    run_git(
        Some(&source),
        &[
            "config",
            &format!("http.{remote}.extraHeader"),
            &format!("Authorization: {bearer}"),
        ],
        "configure request auto merge bearer",
    )
    .unwrap();
    fs::write(source.join("auto-merge.txt"), "merge this head\n").unwrap();
    run_git(
        Some(&source),
        &["add", "auto-merge.txt"],
        "stage auto merge request",
    )
    .unwrap();
    commit_all(&source, "auto merge request");
    run_git(
        Some(&source),
        &[
            "push",
            &remote,
            &format!("HEAD:refs/heads/{AUTO_REQUEST_NAME}"),
        ],
        "push auto merge request",
    )
    .unwrap();
    let request_head = git_head_oid(&source);
    let submitted = api_request(
        app,
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}/submit"),
        Some(&bearer),
        Some("{}"),
    )
    .await;
    assert_eq!(submitted.status(), StatusCode::OK);
    (state, request_id, request_head, server)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorization_is_bound_to_the_exact_revision_and_only_the_current_intent_cancels() {
    let (state, revision_id) = open_request_with_revision(
        AUTO_REQUEST_ID,
        test_owner_id(),
        RequestActorRole::Owner,
        RequestAudience::Private,
    )
    .await;
    let app = router(state);
    let bearer = bearer_header();

    let initial = auto_merge_json(
        app.clone(),
        "GET",
        AUTO_REQUEST_ID,
        &bearer,
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(initial["revision_id"], revision_id);
    assert_eq!(initial["head_oid"], SECOND_HEAD);
    assert!(initial["intent"].is_null());
    assert_eq!(initial["can_enable"], true);

    for body in [
        serde_json::json!({
            "expected_revision_id": "a-different-revision",
            "expected_head_oid": SECOND_HEAD,
        }),
        serde_json::json!({
            "expected_revision_id": revision_id,
            "expected_head_oid": THIRD_HEAD,
        }),
    ] {
        let stale = api_request(
            app.clone(),
            "POST",
            &auto_merge_route(AUTO_REQUEST_ID),
            Some(&bearer),
            Some(&body.to_string()),
        )
        .await;
        assert_eq!(stale.status(), StatusCode::CONFLICT);
    }

    let authorized = authorize(
        app.clone(),
        AUTO_REQUEST_ID,
        &revision_id,
        SECOND_HEAD,
        &bearer,
    )
    .await;
    assert_eq!(authorized["intent"]["status"], "Active");
    assert_eq!(authorized["intent"]["revision_id"], revision_id);
    assert_eq!(authorized["intent"]["head_oid"], SECOND_HEAD);
    assert_eq!(authorized["waiting_reason"], "Waiting for check evaluation");
    assert_eq!(authorized["can_cancel"], true);
    let intent_id = authorized["intent"]["id"].as_str().unwrap();

    let stale_cancel = api_request(
        app.clone(),
        "DELETE",
        &auto_merge_route(AUTO_REQUEST_ID),
        Some(&bearer),
        Some(r#"{"expected_intent_id":"stale-intent"}"#),
    )
    .await;
    assert_eq!(stale_cancel.status(), StatusCode::CONFLICT);

    let canceled = auto_merge_json(
        app,
        "DELETE",
        AUTO_REQUEST_ID,
        &bearer,
        Some(&serde_json::json!({ "expected_intent_id": intent_id }).to_string()),
        StatusCode::OK,
    )
    .await;
    assert_eq!(canceled["intent"]["status"], "Cancelled");
    assert_eq!(canceled["can_cancel"], false);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_authors_cannot_authorize_auto_merge() {
    let author_user_id =
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", PUBLIC_AUTO_SUBJECT);
    let (state, revision_id) = open_request_with_revision(
        "req_public_auto_merge",
        author_user_id,
        RequestActorRole::Public,
        RequestAudience::Public,
    )
    .await;
    let response = api_request(
        router(state),
        "POST",
        &auto_merge_route("req_public_auto_merge"),
        Some(&bearer_header_for(PUBLIC_AUTO_SUBJECT, PUBLIC_AUTO_EMAIL)),
        Some(
            &serde_json::json!({
                "expected_revision_id": revision_id,
                "expected_head_oid": SECOND_HEAD,
            })
            .to_string(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_authorization_reports_pending_checks_and_stops_after_a_new_push() {
    let (state, revision_id) = open_request_with_revision(
        "req_auto_merge_pending",
        test_owner_id(),
        RequestActorRole::Owner,
        RequestAudience::Private,
    )
    .await;
    record_queued_request_check(&state, "req_auto_merge_pending").await;
    let app = router(state.clone());
    let bearer = bearer_header();

    let pending = authorize(
        app.clone(),
        "req_auto_merge_pending",
        &revision_id,
        SECOND_HEAD,
        &bearer,
    )
    .await;
    assert_eq!(pending["intent"]["status"], "Active");
    assert_eq!(pending["waiting_reason"], "Waiting for checks to finish");

    record_revision(
        &state,
        "req_auto_merge_pending",
        &test_owner_id(),
        Some(SECOND_HEAD),
        THIRD_HEAD,
        "event_req_auto_merge_pending_revision_2",
        unix_now(),
    )
    .await;
    crate::use_cases::request_auto_merge::reconcile_once(&state, unix_now() + 10)
        .await
        .unwrap();

    let stopped = auto_merge_json(
        app,
        "GET",
        "req_auto_merge_pending",
        &bearer,
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(stopped["intent"]["status"], "Stopped");
    assert_eq!(stopped["intent"]["reason"], "RequestChanged");
    assert_eq!(
        stopped["revision_id"],
        "event_req_auto_merge_pending_revision_2"
    );
    assert_eq!(stopped["head_oid"], THIRD_HEAD);

    record_revision(
        &state,
        "req_auto_merge_pending",
        &test_owner_id(),
        Some(THIRD_HEAD),
        SECOND_HEAD,
        "event_req_auto_merge_pending_revision_3",
        unix_now(),
    )
    .await;
    let returned_to_authorized_head = auto_merge_json(
        router(state),
        "GET",
        "req_auto_merge_pending",
        &bearer,
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(returned_to_authorized_head["head_oid"], SECOND_HEAD);
    assert_eq!(returned_to_authorized_head["intent"]["status"], "Stopped");
    assert_eq!(
        returned_to_authorized_head["intent"]["reason"],
        "RequestChanged"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configuration_failure_stays_stopped_if_the_evaluation_later_clears() {
    let request_id = "req_auto_merge_failed_checks";
    let (state, revision_id) = open_request_with_revision(
        request_id,
        test_owner_id(),
        RequestActorRole::Owner,
        RequestAudience::Private,
    )
    .await;
    state
        .metadata
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            evaluation: RequestCheckEvaluation::configuration_error(
                request_id,
                SECOND_HEAD,
                "invalid request workflow",
                5,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
        })
        .await
        .unwrap();
    let app = router(state.clone());
    let bearer = bearer_header();
    let active = authorize(app.clone(), request_id, &revision_id, SECOND_HEAD, &bearer).await;
    assert_eq!(active["intent"]["status"], "Active");

    let attempt_time = unix_now() + 1;
    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&state, attempt_time)
            .await
            .unwrap(),
        1
    );
    let stopped = auto_merge_json(
        app.clone(),
        "GET",
        request_id,
        &bearer,
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(stopped["intent"]["status"], "Stopped");
    assert_eq!(stopped["intent"]["reason"], "ChecksConfigurationError");

    state
        .metadata
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            evaluation: RequestCheckEvaluation::no_checks(
                request_id,
                SECOND_HEAD,
                attempt_time + 1,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&state, attempt_time + 2)
            .await
            .unwrap(),
        0
    );
    let still_stopped =
        auto_merge_json(app, "GET", request_id, &bearer, None, StatusCode::OK).await;
    assert_eq!(still_stopped["intent"]["status"], "Stopped");
    assert_eq!(
        still_stopped["intent"]["reason"],
        "ChecksConfigurationError"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_run_failure_stays_stopped_after_retry() {
    let request_id = "req_auto_merge_run_retry";
    let (state, revision_id) = open_request_with_revision(
        request_id,
        test_owner_id(),
        RequestActorRole::Owner,
        RequestAudience::Private,
    )
    .await;
    let run_id = record_queued_request_check(&state, request_id).await;
    let app = router(state.clone());
    let bearer = bearer_header();
    let active = authorize(app.clone(), request_id, &revision_id, SECOND_HEAD, &bearer).await;
    assert_eq!(active["intent"]["status"], "Active");
    assert_eq!(active["waiting_reason"], "Waiting for checks to finish");

    let cancellation_time = unix_now();
    let canceled = state
        .metadata
        .runs()
        .request_run_cancellation(&test_owner_id(), TEST_REPO_ID, &run_id, cancellation_time)
        .await
        .unwrap();
    assert_eq!(canceled.state, RunState::Canceled);
    let stopped = auto_merge_json(
        app.clone(),
        "GET",
        request_id,
        &bearer,
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(stopped["intent"]["status"], "Stopped");
    assert_eq!(stopped["intent"]["reason"], "ChecksFailed");

    let retried = state
        .metadata
        .runs()
        .retry_run(
            &test_owner_id(),
            TEST_REPO_ID,
            &run_id,
            cancellation_time + 1,
        )
        .await
        .unwrap();
    assert_eq!(retried.state, RunState::Queued);
    let still_stopped =
        auto_merge_json(app, "GET", request_id, &bearer, None, StatusCode::OK).await;
    assert_eq!(still_stopped["intent"]["status"], "Stopped");
    assert_eq!(still_stopped["intent"]["reason"], "ChecksFailed");
    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&state, cancellation_time + 2)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_merge_fulfills_an_active_authorization() {
    let (state, request_id, request_head, _server) =
        native_open_request("request-auto-merge-manual").await;
    let app = router(state);
    let bearer = bearer_header();
    let ready = auto_merge_json(
        app.clone(),
        "GET",
        &request_id,
        &bearer,
        None,
        StatusCode::OK,
    )
    .await;
    let revision_id = ready["revision_id"].as_str().unwrap();
    let active = authorize(
        app.clone(),
        &request_id,
        revision_id,
        &request_head,
        &bearer,
    )
    .await;
    assert_eq!(active["intent"]["status"], "Active");

    let merged = api_request(
        app.clone(),
        "POST",
        &scope_api_contract::routes::repo_request_merge(
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            &request_id,
        ),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(merged.status(), StatusCode::OK);

    let fulfilled = auto_merge_json(app, "GET", &request_id, &bearer, None, StatusCode::OK).await;
    assert_eq!(fulfilled["intent"]["status"], "Fulfilled");
    assert_eq!(fulfilled["intent"]["head_oid"], request_head);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn successful_reconciliation_merges_the_authorized_head_once() {
    let (state, request_id, request_head, _server) =
        native_open_request("request-auto-merge-native").await;
    let app = router(state.clone());
    let bearer = bearer_header();
    let ready = auto_merge_json(
        app.clone(),
        "GET",
        &request_id,
        &bearer,
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(ready["head_oid"], request_head);
    assert_eq!(ready["waiting_reason"], serde_json::Value::Null);
    let revision_id = ready["revision_id"].as_str().unwrap();
    let commit_count_before = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .graph
        .commits
        .len();

    let scheduled = authorize(
        app.clone(),
        &request_id,
        revision_id,
        &request_head,
        &bearer,
    )
    .await;
    assert_eq!(scheduled["intent"]["status"], "Active");
    assert_eq!(scheduled["intent"]["head_oid"], request_head);
    assert_eq!(scheduled["can_cancel"], true);

    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&state, unix_now())
            .await
            .unwrap(),
        1
    );
    let fulfilled = auto_merge_json(app, "GET", &request_id, &bearer, None, StatusCode::OK).await;
    assert_eq!(fulfilled["intent"]["status"], "Fulfilled");
    assert_eq!(fulfilled["intent"]["head_oid"], request_head);
    assert_eq!(fulfilled["can_cancel"], false);
    assert_eq!(
        live_file_content(&state, "/auto-merge.txt")
            .await
            .as_deref(),
        Some("merge this head\n")
    );
    assert_eq!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .graph
            .commits
            .len(),
        commit_count_before + 1
    );

    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&state, unix_now())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .graph
            .commits
            .len(),
        commit_count_before + 1
    );
}
