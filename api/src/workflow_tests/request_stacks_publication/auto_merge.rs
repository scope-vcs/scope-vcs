use super::*;
use scope_domain::requests::{RequestAutoMergeIntentStatus, RequestState};

async fn auto_merge_response(state: &AppState, request_id: &str) -> serde_json::Value {
    expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &scope_api_contract::routes::repo_request_auto_merge(
                TEST_REPO_OWNER,
                TEST_REPO_NAME,
                request_id,
            ),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await
}

async fn authorize_layer(state: &AppState, request_id: &str) -> serde_json::Value {
    let revision = state
        .metadata
        .requests()
        .latest_request_revision(request_id)
        .await
        .unwrap()
        .unwrap();
    expect_json(
        api_request(
            router(state.clone()),
            "POST",
            &scope_api_contract::routes::repo_request_auto_merge(
                TEST_REPO_OWNER,
                TEST_REPO_NAME,
                request_id,
            ),
            Some(&bearer_header()),
            Some(
                &serde_json::json!({
                    "expected_revision_id": revision.id,
                    "expected_head_oid": revision.new_head_oid,
                })
                .to_string(),
            ),
        )
        .await,
        StatusCode::OK,
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_authorizations_wait_then_land_three_layers_with_a_parent_extension() {
    let fixture = PrivateStackFixture::create("private-stack-auto-chain").await;
    let child = stack_request(&fixture.state, &fixture.child_id).await;
    let child_revision = fixture.latest_child_revision().await;
    let grandchild_id = start_private_request(
        &fixture.state,
        "stack-grandchild",
        Some(serde_json::json!({
            "kind": "request",
            "request_id": child.id,
            "revision_id": child_revision.id,
            "head_oid": child.head_oid,
        })),
    )
    .await;
    commit_file(&fixture.source, "grandchild.txt", "grandchild layer\n");
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        "stack-grandchild",
        Some(&fixture.child_head),
        &[],
    );
    submit_private_request(&fixture.state, &grandchild_id).await;
    let grandchild_revision = fixture
        .state
        .metadata
        .requests()
        .latest_request_revision(&grandchild_id)
        .await
        .unwrap()
        .unwrap();
    let mut authorizations = Vec::new();
    for request_id in [&fixture.child_id, &grandchild_id] {
        let ready = auto_merge_response(&fixture.state, request_id).await;
        assert_eq!(ready["can_enable"], true);
        let enabled = authorize_layer(&fixture.state, request_id).await;
        assert_eq!(enabled["intent"]["status"], "Active");
        assert_eq!(
            enabled["waiting_reason"],
            "Waiting for the parent request to land"
        );
        authorizations.push(enabled["intent"]["id"].as_str().unwrap().to_string());
    }
    let child_checks = fixture
        .state
        .metadata
        .requests()
        .request_check_evaluation(&fixture.child_id, &fixture.child_head)
        .await
        .unwrap()
        .unwrap();
    let now = unix_now();
    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&fixture.state, now)
            .await
            .unwrap(),
        2
    );
    for request_id in [&fixture.parent_id, &fixture.child_id, &grandchild_id] {
        assert_eq!(
            stack_request(&fixture.state, request_id).await.state(),
            RequestState::Open
        );
    }
    let extended_parent = fixture.extend_parent().await;
    let extended_child = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(
        extended_child.stack_status,
        RequestStackStatus::ParentExtended
    );
    assert_eq!(extended_child.base, child.base);
    assert_eq!(fixture.latest_child_revision().await, child_revision);
    assert_eq!(
        fixture
            .state
            .metadata
            .requests()
            .request_check_evaluation(&fixture.child_id, &fixture.child_head)
            .await
            .unwrap()
            .unwrap(),
        child_checks
    );
    let parent_authorization = authorize_layer(&fixture.state, &fixture.parent_id).await;
    assert_eq!(parent_authorization["head_oid"], extended_parent);
    let restarted = fixture.state.clone();
    for offset in [10, 20, 30] {
        crate::use_cases::request_auto_merge::reconcile_once(&restarted, now + offset)
            .await
            .unwrap();
    }
    let parent = stack_request(&fixture.state, &fixture.parent_id).await;
    let landed_child = stack_request(&fixture.state, &fixture.child_id).await;
    let grandchild = stack_request(&fixture.state, &grandchild_id).await;
    for request in [&parent, &landed_child, &grandchild] {
        assert_eq!(request.state(), RequestState::Merged);
    }
    for (request_id, authorization_id) in [&fixture.child_id, &grandchild_id]
        .into_iter()
        .zip(authorizations)
    {
        let intent = fixture
            .state
            .metadata
            .requests()
            .request_auto_merge_intent(request_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(intent.id, authorization_id);
        assert_eq!(intent.status, RequestAutoMergeIntentStatus::Fulfilled);
    }
    assert_eq!(landed_child.base.review_base_oid(), fixture.parent_pin);
    assert_eq!(grandchild.base.review_base_oid(), fixture.child_head);
    assert_eq!(fixture.latest_child_revision().await, child_revision);
    assert_eq!(
        fixture
            .state
            .metadata
            .requests()
            .latest_request_revision(&grandchild_id)
            .await
            .unwrap()
            .unwrap(),
        grandchild_revision
    );
    fetch_stack_main(&fixture);
    for (ancestor, descendant) in [
        (
            parent.merged_main_oid.as_deref().unwrap(),
            landed_child.merged_main_oid.as_deref().unwrap(),
        ),
        (
            landed_child.merged_main_oid.as_deref().unwrap(),
            grandchild.merged_main_oid.as_deref().unwrap(),
        ),
    ] {
        assert!(
            run_git_output(
                Some(&fixture.source),
                &["merge-base", "--is-ancestor", ancestor, descendant],
                "verify ordered stack landing"
            )
            .unwrap()
            .status
            .success()
        );
    }
    for (path, content) in [
        ("/parent-fixup.txt", "parent extension\n"),
        ("/child.txt", "child layer\n"),
        ("/grandchild.txt", "grandchild layer\n"),
    ] {
        assert_eq!(
            live_file_content(&fixture.state, path).await.as_deref(),
            Some(content)
        );
    }
    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&restarted, now + 40)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_authorization_never_authorizes_its_child() {
    let fixture = PrivateStackFixture::create("private-stack-no-inherited-auto-merge").await;
    authorize_layer(&fixture.state, &fixture.parent_id).await;
    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&fixture.state, unix_now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        stack_request(&fixture.state, &fixture.parent_id)
            .await
            .state(),
        RequestState::Merged
    );
    assert_eq!(
        stack_request(&fixture.state, &fixture.child_id)
            .await
            .state(),
        RequestState::Open
    );
    assert!(
        fixture
            .state
            .metadata
            .requests()
            .request_auto_merge_intent(&fixture.child_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&fixture.state, unix_now() + 10)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retargeted_child_requires_fresh_github_confirmation_before_automatic_landing() {
    use super::super::fake_github::{GITHUB_REPOSITORY_ID, check_run};
    use super::super::git_request_refs::github_checks::{
        REQUIRED_CHECK, connect_github, push_pass,
    };
    use std::sync::atomic::Ordering;

    let mut fixture = PrivateStackFixture::create("private-stack-fresh-auto-checks").await;
    let fake = connect_github(&mut fixture.state, &[REQUIRED_CHECK]).await;
    let repo = fixture
        .state
        .metadata
        .repositories()
        .repository_record(TEST_REPO_ID)
        .await
        .unwrap()
        .unwrap();
    for request_id in [&fixture.parent_id, &fixture.child_id] {
        fixture
            .state
            .metadata
            .requests()
            .forget_request_check_evaluations_for_tests(request_id)
            .await
            .unwrap();
        let request = stack_request(&fixture.state, request_id).await;
        crate::use_cases::request_checks::checks_view(&fixture.state, &repo, &request)
            .await
            .unwrap();
    }
    push_pass(&fixture.state, unix_now()).await;
    for (run_id, head) in [(1, &fixture.parent_pin), (2, &fixture.child_head)] {
        fake.report_check_runs(
            head,
            vec![check_run(run_id, REQUIRED_CHECK, head, Some("success"))],
        );
        crate::use_cases::github_check_results::refresh_checks_for_delivery(
            &fixture.state,
            GITHUB_REPOSITORY_ID,
            head,
        )
        .await
        .unwrap();
    }
    let enabled = authorize_layer(&fixture.state, &fixture.child_id).await;
    assert_eq!(enabled["intent"]["status"], "Active");
    let reviewed_checks = fixture
        .state
        .metadata
        .requests()
        .request_check_evaluation(&fixture.child_id, &fixture.child_head)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reviewed_checks.tested_oid, fixture.child_head);
    assert_eq!(
        merge_stack_request(&fixture.state, &fixture.parent_id, &fixture.parent_pin)
            .await
            .status(),
        StatusCode::OK
    );
    let retargeted_checks = fixture
        .state
        .metadata
        .requests()
        .request_check_evaluation(&fixture.child_id, &fixture.child_head)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retargeted_checks.id, reviewed_checks.id);
    assert_eq!(
        retargeted_checks.review_base_oid,
        reviewed_checks.review_base_oid
    );
    assert_eq!(retargeted_checks.tested_oid, reviewed_checks.tested_oid);
    assert_eq!(retargeted_checks.revision_id, reviewed_checks.revision_id);
    fixture
        .state
        .metadata
        .requests()
        .age_github_check_reads_for_tests(TEST_REPO_ID, 120)
        .await
        .unwrap();
    fake.report_check_runs(
        &fixture.child_head,
        vec![
            check_run(2, REQUIRED_CHECK, &fixture.child_head, Some("success")),
            check_run(3, REQUIRED_CHECK, &fixture.child_head, Some("failure")),
        ],
    );
    let reads_before = fake.check_run_reads.load(Ordering::SeqCst);
    let now = unix_now();
    assert_eq!(
        crate::use_cases::request_auto_merge::reconcile_once(&fixture.state, now)
            .await
            .unwrap(),
        1
    );
    assert!(fake.check_run_reads.load(Ordering::SeqCst) > reads_before);
    crate::use_cases::request_auto_merge::reconcile_once(&fixture.state, now + 20)
        .await
        .unwrap();
    let response = auto_merge_response(&fixture.state, &fixture.child_id).await;
    assert_eq!(response["intent"]["status"], "Stopped");
    assert_eq!(response["intent"]["reason"], "ChecksFailed");
    assert_eq!(response["intent"]["id"], enabled["intent"]["id"]);
    assert_eq!(
        stack_request(&fixture.state, &fixture.child_id)
            .await
            .state(),
        RequestState::Open
    );
    assert!(
        live_file_content(&fixture.state, "/child.txt")
            .await
            .is_none()
    );
    assert_eq!(
        live_file_content(&fixture.state, "/parent.txt")
            .await
            .as_deref(),
        Some("parent layer\n")
    );
}
