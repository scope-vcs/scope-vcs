use super::*;
use scope_api_contract::routes::repo_request_checks_approve;
use scope_domain::requests::{NativeRequestCheck, RequestCheck};

fn approve_route(request_id: &str) -> String {
    repo_request_checks_approve(TEST_REPO_OWNER, TEST_REPO_NAME, request_id)
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
        "CI is waiting for a maintainer to start it"
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
        "CI has not finished"
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
