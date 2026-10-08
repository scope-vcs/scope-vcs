use super::*;
use scope_api_contract::routes::{repo_request, repo_request_checks, repo_request_checks_approve};
use scope_domain::requests::{NativeRequestCheck, RequestCheck, RequestCheckEvaluation};
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

async fn owner_request_push(
    label: &str,
    workflows: &[(&str, String)],
) -> (AppState, String, TestServer) {
    owner_request_push_with_native_runs(label, workflows, true).await
}

async fn owner_request_push_with_native_runs(
    label: &str,
    workflows: &[(&str, String)],
    native_runs: bool,
) -> (AppState, String, TestServer) {
    let (state, source, _base_head) =
        super::super::push_intent_completion::published_git_fixture(label).await;
    if native_runs {
        allow_native_runs(&state).await;
    }
    let app = router(state.clone());
    let bearer = bearer_header();
    let started = api_request(
        app.clone(),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer),
        Some(r#"{"name":"checks","view":"private"}"#),
    )
    .await;
    let request_id = expect_json(started, StatusCode::OK).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (origin, server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
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
    assert_eq!(checks["checks"][0]["provider"], "native");
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

    state
        .metadata
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: state
                .metadata
                .requests()
                .request_check_base(TEST_REPO_ID)
                .await
                .unwrap(),
            repository_incarnation: state
                .metadata
                .repositories()
                .repository_record(TEST_REPO_ID)
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::configuration_error(
                &request_id,
                "ffffffffffffffffffffffffffffffffffffffff",
                "stale head configuration error",
                unix_now(),
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap_err();
    let app = router(state);
    let list = expect_json(
        api_request(
            app.clone(),
            "GET",
            &format!("/v1/repos/{TEST_REPO_ID}/requests"),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let listed = list["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|request| request["id"] == request_id)
        .unwrap();
    assert_eq!(listed["mergeability"]["status"], "ChecksPending");

    let queue = expect_json(
        api_request(
            app,
            "GET",
            &format!("/v1/repos/{TEST_REPO_ID}/requests/queue?section=active"),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let queued = queue["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["request"]["id"] == request_id)
        .unwrap();
    assert_eq!(queued["request"]["mergeability"]["status"], "ChecksPending");
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

async fn forget_evaluations(state: &AppState, request_id: &str) {
    state
        .metadata
        .requests()
        .forget_request_check_evaluations_for_tests(request_id)
        .await
        .unwrap();
}

async fn checks(state: &AppState, request_id: &str, bearer: &str) -> serde_json::Value {
    expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &checks_route(request_id),
            Some(bearer),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await
}

async fn listed_status(state: &AppState, request_id: &str) -> serde_json::Value {
    let list = expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &format!("/v1/repos/{TEST_REPO_ID}/requests"),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    list["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|request| request["id"] == request_id)
        .unwrap()["mergeability"]["status"]
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_head_whose_evaluation_failed_is_recovered_without_a_viewer() {
    let (state, request_id, _server) = owner_request_push(
        "request-checks-unevaluated",
        &[(".scope/runs/checks.yml", request_workflow())],
    )
    .await;
    forget_evaluations(&state, &request_id).await;
    insert_member_user(&state).await;
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);

    assert_eq!(
        listed_status(&state, &request_id).await,
        "ChecksNotEvaluated"
    );
    assert_eq!(
        listed_status(&state, &request_id).await,
        "ChecksNotEvaluated"
    );

    let read = checks(&state, &request_id, &member).await;
    assert_eq!(read["state"], serde_json::Value::Null);
    assert_eq!(read["mergeability"]["status"], "ChecksNotEvaluated");
    assert!(
        state
            .metadata
            .requests()
            .request_check_evaluation(&request_id, read["head_oid"].as_str().unwrap())
            .await
            .unwrap()
            .is_none()
    );
    let mut recovery_cursor = Default::default();
    assert_eq!(
        crate::use_cases::request_checks::reconcile_request_checks_once(
            &state,
            &mut recovery_cursor
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        crate::use_cases::request_checks::reconcile_request_checks_once(
            &state,
            &mut recovery_cursor
        )
        .await
        .unwrap(),
        0
    );
    let looked = checks(&state, &request_id, &member).await;
    assert_eq!(looked["state"], "started");
    assert_eq!(looked["mergeability"]["status"], "ChecksPending");
    let run_id = looked["checks"][0]["run_id"].as_str().unwrap().to_string();
    let run = state.metadata.runs().run(&run_id).await.unwrap().unwrap();
    assert_eq!(run.requested_by_user_id.as_deref(), Some(&*test_owner_id()));

    let state_response = expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &scope_api_contract::routes::repo_request_state(
                TEST_REPO_OWNER,
                TEST_REPO_NAME,
                &request_id,
            ),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(state_response["viewer"]["id"], test_owner_id());
    assert_eq!(
        state_response["detail"]["request"]["head_oid"],
        looked["head_oid"]
    );
    assert_eq!(state_response["checks"]["head_oid"], looked["head_oid"]);
    assert_eq!(state_response["auto_merge"]["head_oid"], looked["head_oid"]);
    assert_eq!(
        state_response["detail"]["request"]["mergeability"],
        state_response["checks"]["mergeability"]
    );
    assert_eq!(state_response["auto_merge"]["can_enable"], true);
    assert_eq!(
        state_response["checks"]["checks"][0]["run_id"],
        run_id.as_str()
    );
    let hidden = api_request(
        router(state.clone()),
        "GET",
        &scope_api_contract::routes::repo_request_state(
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            &request_id,
        ),
        None,
        None,
    )
    .await;
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
    let again = checks(&state, &request_id, &member).await;
    assert_eq!(again["checks"][0]["run_id"], run_id.as_str());
    assert_eq!(again["checks"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn merging_an_unevaluated_head_evaluates_it_instead() {
    let (state, request_id, _server) = owner_request_push(
        "request-checks-unevaluated-merge",
        &[(".scope/runs/checks.yml", request_workflow())],
    )
    .await;
    forget_evaluations(&state, &request_id).await;

    let merge = api_request(
        router(state.clone()),
        "POST",
        &merge_route(&request_id),
        Some(&bearer_header()),
        None,
    )
    .await;

    assert_eq!(
        expect_json(merge, StatusCode::CONFLICT).await["message"],
        "checks have not finished"
    );
    assert_eq!(listed_status(&state, &request_id).await, "ChecksPending");
}

async fn record_awaiting_approval(state: &AppState, request_id: &str) {
    allow_native_runs(state).await;
    forget_evaluations(state, request_id).await;
    let request = stored_request(state, request_id).await;
    let workflow = request_workflow();
    let revisions = scope_run_config::parse_workflow_set(
        TEST_REPO_ID,
        [("/.scope/runs/checks.yml", workflow.as_bytes())],
    )
    .unwrap();
    let checks = revisions
        .iter()
        .map(|revision| RequestCheck::Native(NativeRequestCheck::for_revision(revision)))
        .collect();
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
            expected_canonical_main_oid: state
                .metadata
                .requests()
                .request_check_base(TEST_REPO_ID)
                .await
                .unwrap(),
            repository_incarnation: state
                .metadata
                .repositories()
                .repository_record(TEST_REPO_ID)
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation,
            revisions,
            runs: Vec::new(),
            push_to_github: false,
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
        Some(&reviewed_head_body(&state, REQUEST_ID).await),
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
            Some(&reviewed_head_body(&state, REQUEST_ID).await),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recorded_checks_can_be_approved_by_a_maintainer_after_the_request_closes() {
    let (state, _owner_source) =
        test_state_with_mergeable_request("closed-request-checks-approval").await;
    insert_member_user(&state).await;
    let (_source, _remote, _server, _head) =
        request_checkout(&state, "closed-request-checks-approval-contributor").await;
    record_awaiting_approval(&state, REQUEST_ID).await;
    let app = router(state.clone());
    let public = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    submit(app.clone(), REQUEST_ID, &public).await;

    let closed = expect_json(
        api_request(
            app.clone(),
            "DELETE",
            &repo_request(TEST_REPO_OWNER, TEST_REPO_NAME, REQUEST_ID),
            Some(&public),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(closed["deleted"], false);

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
    assert_eq!(waiting["mergeability"]["status"], "Closed");

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
    assert_eq!(contributor_view["can_approve"], false);

    let refused = api_request(
        app.clone(),
        "POST",
        &approve_route(REQUEST_ID),
        Some(&public),
        Some(&reviewed_head_body(&state, REQUEST_ID).await),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    let approved = expect_json(
        api_request(
            app,
            "POST",
            &approve_route(REQUEST_ID),
            Some(&member),
            Some(&reviewed_head_body(&state, REQUEST_ID).await),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(approved["state"], "started");
    assert_eq!(approved["can_approve"], false);
    assert_eq!(approved["checks"][0]["run_state"], "queued");
    assert_eq!(approved["mergeability"]["status"], "Closed");
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unlisted_owners_request_workflows_are_ignored() {
    let (state, request_id, _server) = owner_request_push_with_native_runs(
        "request-checks-unlisted",
        &[(".scope/runs/checks.yml", request_workflow())],
        false,
    )
    .await;

    let checks = checks(&state, &request_id, &bearer_header()).await;

    assert_eq!(checks["state"], "no-checks");
    assert!(checks["checks"].as_array().unwrap().is_empty());
    assert_eq!(checks["mergeability"]["status"], "Ready");
    assert!(
        state
            .metadata
            .runs()
            .repository_run_history_page(scope_postgres::db::RunHistoryPageQuery {
                repository_id: TEST_REPO_ID,
                workflow_path: None,
                git_oid: None,
                after: None,
                limit: 10,
            })
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_the_owner_stops_admission_and_ends_the_wait_on_checks() {
    let (mut state, request_id, _server) = owner_request_push(
        "request-checks-withdrawn",
        &[(".scope/runs/checks.yml", request_workflow())],
    )
    .await;
    let started = checks(&state, &request_id, &bearer_header()).await;
    assert_eq!(started["state"], "started");
    let run_id = started["checks"][0]["run_id"].as_str().unwrap().to_string();

    state.operator_token = Some(Arc::from("operator-secret"));
    let removal = expect_json(
        api_request(
            router(state.clone()),
            "DELETE",
            &format!("/v1/admin/native-runs/accounts/{TEST_REPO_OWNER}"),
            Some("Bearer operator-secret"),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(removal["removed"], true);
    assert_eq!(removal["withdrawn_request_ids"][0], request_id.as_str());
    assert_eq!(removal["canceled_run_ids"][0], run_id.as_str());

    let withdrawn = checks(&state, &request_id, &bearer_header()).await;
    assert_eq!(withdrawn["state"], "configuration-error");
    assert_eq!(
        withdrawn["message"],
        scope_domain::runs::availability::NATIVE_RUNS_UNAVAILABLE
    );
    assert_eq!(
        withdrawn["mergeability"]["status"],
        "ChecksConfigurationError"
    );
    assert_eq!(
        listed_status(&state, &request_id).await,
        "ChecksConfigurationError"
    );
    let run = state.metadata.runs().run(&run_id).await.unwrap().unwrap();
    assert_eq!(run.state, scope_domain::runs::run::RunState::Canceled);
    let now = unix_now();
    assert!(matches!(
        state
            .metadata
            .runs()
            .admit_next_job(
                10,
                "attempt-withdrawn",
                &"e".repeat(64),
                "runtime",
                now,
                now + 60
            )
            .await
            .unwrap(),
        scope_postgres::db::DispatchAdmission::Empty
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_merge_rejects_main_that_moved_after_authorization() {
    let (state, request_id, _server) = owner_request_push(
        "request-merge-moved-main",
        &[(".scope/runs/manual.yml", WORKFLOW.to_string())],
    )
    .await;
    forget_evaluations(&state, &request_id).await;
    state
        .metadata
        .admin()
        .execute_for_tests(
            "CREATE FUNCTION bump_repository_version() RETURNS trigger AS $$
             BEGIN
                 UPDATE scope_repositories SET change_version = change_version + 1
                 WHERE id = (SELECT repo_id FROM scope_requests WHERE id = NEW.request_id);
                 RETURN NEW;
             END $$ LANGUAGE plpgsql;
             CREATE TRIGGER bump_repository_version
             AFTER INSERT ON scope_request_check_evaluations
             FOR EACH ROW EXECUTE FUNCTION bump_repository_version();",
        )
        .await
        .unwrap();

    let merge = api_request(
        router(state.clone()),
        "POST",
        &merge_route(&request_id),
        Some(&bearer_header()),
        None,
    )
    .await;

    assert_eq!(
        expect_json(merge, StatusCode::CONFLICT).await["message"],
        "repository changed while the merge was checked; retry"
    );
    assert_eq!(listed_status(&state, &request_id).await, "Ready");
}
