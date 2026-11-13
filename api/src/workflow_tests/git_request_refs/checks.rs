use super::github_checks::checks;
use super::*;
use scope_api_contract::routes::repo_request;

mod approval;

fn merge_route(request_id: &str) -> String {
    scope_api_contract::routes::repo_request_merge(TEST_REPO_OWNER, TEST_REPO_NAME, request_id)
}

async fn owner_request_push(label: &str) -> (AppState, String, TestServer) {
    let (state, source, _base_head) =
        super::super::push_intent_completion::published_git_fixture(label).await;
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
    push_change(
        &source,
        &remote,
        "refs/heads/checks",
        "request.txt",
        "request work\n",
        "request change",
    )
    .unwrap();
    submit_request(app, &request_id, &bearer).await;
    (state, request_id, server)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heads_in_a_repository_without_github_owe_no_checks() {
    let (state, request_id, _server) = owner_request_push("request-checks-none").await;

    let checks = checks(&state, &request_id, &bearer_header()).await;

    assert_eq!(checks["state"], "no-checks");
    assert_eq!(checks["outcome"], "clear");
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
    let (state, request_id, _server) = owner_request_push("request-checks-unevaluated").await;
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
    assert_eq!(looked["state"], "no-checks");
    assert_eq!(looked["mergeability"]["status"], "Ready");

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
    assert_eq!(checks(&state, &request_id, &member).await, looked);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_merge_rechecks_a_repository_that_moved_after_authorization() {
    let (state, request_id, _server) = owner_request_push("request-merge-moved-main").await;
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
        expect_json(merge, StatusCode::OK).await["request"]["state"],
        "Merged"
    );
}
