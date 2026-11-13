use super::super::github_checks::{REQUIRED_CHECK, connect_github, push_pass};
use super::*;
use scope_api_contract::routes::repo_request_checks_approve;

fn approve_route(request_id: &str) -> String {
    repo_request_checks_approve(TEST_REPO_OWNER, TEST_REPO_NAME, request_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checks_awaiting_approval_wait_for_a_maintainer_and_gate_the_merge() {
    let (mut state, _owner_source) =
        test_state_with_mergeable_request("request-checks-approval").await;
    connect_github(&mut state, &[REQUIRED_CHECK]).await;
    insert_member_user(&state).await;
    let (_source, _remote, _server, _head) =
        request_checkout(&state, "request-checks-approval-contributor").await;
    let app = router(state.clone());
    let public = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    submit_request(app.clone(), REQUEST_ID, &public).await;

    let waiting = checks(&state, REQUEST_ID, &member).await;
    assert_eq!(waiting["state"], "awaiting-approval");
    assert_eq!(waiting["can_approve"], true);
    assert_eq!(waiting["checks"].as_array().unwrap().len(), 1);
    assert_eq!(waiting["checks"][0]["name"], REQUIRED_CHECK);
    assert_eq!(waiting["checks"][0]["status"], serde_json::Value::Null);
    assert_eq!(waiting["mergeability"]["status"], "ChecksAwaitingApproval");

    let contributor_view = checks(&state, REQUEST_ID, &public).await;
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
    assert_eq!(push_pass(&state, unix_now()).await, 0);

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
    assert_eq!(approved["mergeability"]["status"], "ChecksPending");
    assert_eq!(push_pass(&state, unix_now()).await, 1);

    let pending_merge =
        api_request(app, "POST", &merge_route(REQUEST_ID), Some(&member), None).await;
    assert_eq!(
        expect_json(pending_merge, StatusCode::CONFLICT).await["message"],
        "CI has not finished"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recorded_checks_can_be_approved_by_a_maintainer_after_the_request_closes() {
    let (mut state, _owner_source) =
        test_state_with_mergeable_request("closed-request-checks-approval").await;
    connect_github(&mut state, &[REQUIRED_CHECK]).await;
    insert_member_user(&state).await;
    let (_source, _remote, _server, _head) =
        request_checkout(&state, "closed-request-checks-approval-contributor").await;
    let app = router(state.clone());
    let public = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    submit_request(app.clone(), REQUEST_ID, &public).await;

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

    let waiting = checks(&state, REQUEST_ID, &member).await;
    assert_eq!(waiting["state"], "awaiting-approval");
    assert_eq!(waiting["can_approve"], true);
    assert_eq!(waiting["mergeability"]["status"], "Closed");

    let contributor_view = checks(&state, REQUEST_ID, &public).await;
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
    assert_eq!(approved["mergeability"]["status"], "Closed");
    assert_eq!(push_pass(&state, unix_now()).await, 0);
}
