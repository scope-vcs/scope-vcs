use super::*;
use scope_domain::views::ViewId;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconciliation_evaluates_a_saved_intent_after_restart_without_a_viewer() {
    let (state, _source, _remote, request_id, request_head, _server) =
        native_open_request("request-auto-merge-recover-checks", ViewId::private()).await;
    let revision = state
        .metadata
        .requests()
        .latest_request_revision(&request_id)
        .await
        .unwrap()
        .unwrap();
    state
        .metadata
        .requests()
        .forget_request_check_evaluations_for_tests(&request_id)
        .await
        .unwrap();
    crate::use_cases::request_auto_merge::authorize(
        &state,
        &request_id,
        &test_owner_id(),
        revision.id,
        request_head.clone(),
    )
    .await
    .unwrap();
    assert!(
        state
            .metadata
            .requests()
            .request_check_evaluation(&request_id, &request_head)
            .await
            .unwrap()
            .is_none()
    );

    let restarted = state.clone();
    assert_eq!(reconcile(&restarted, unix_now()).await, 1);
    assert!(
        restarted
            .metadata
            .requests()
            .request_check_evaluation(&request_id, &request_head)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        restarted
            .metadata
            .requests()
            .request_auto_merge_intent(&request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        scope_domain::requests::RequestAutoMergeIntentStatus::Fulfilled
    );
    assert_eq!(reconcile(&restarted, unix_now() + 1).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconciliation_retries_a_transient_check_evaluation_failure() {
    let (state, _source, _remote, request_id, request_head, _server) =
        native_open_request("request-auto-merge-retry-checks", ViewId::public()).await;
    allow_native_runs(&state).await;
    let revision = state
        .metadata
        .requests()
        .latest_request_revision(&request_id)
        .await
        .unwrap()
        .unwrap();
    state
        .metadata
        .requests()
        .forget_request_check_evaluations_for_tests(&request_id)
        .await
        .unwrap();
    let catalog_source = state
        .metadata
        .repositories()
        .current_repository_workflow_catalog(TEST_REPO_ID)
        .await
        .unwrap()
        .unwrap()
        .catalog
        .unwrap()
        .source_head_oid()
        .to_string();
    state
        .metadata
        .repositories()
        .set_repository_workflow_catalog_source_for_tests(
            TEST_REPO_ID,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .await
        .unwrap();
    crate::use_cases::request_auto_merge::authorize(
        &state,
        &request_id,
        &test_owner_id(),
        revision.id,
        request_head.clone(),
    )
    .await
    .unwrap();
    assert!(
        state
            .metadata
            .requests()
            .request_check_evaluation(&request_id, &request_head)
            .await
            .unwrap()
            .is_none()
    );

    let first_attempt = unix_now();
    assert_eq!(reconcile(&state, first_attempt).await, 1);
    assert!(
        state
            .metadata
            .requests()
            .request_check_evaluation(&request_id, &request_head)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(reconcile(&state, first_attempt + 5).await, 0);
    state
        .metadata
        .repositories()
        .set_repository_workflow_catalog_source_for_tests(TEST_REPO_ID, &catalog_source)
        .await
        .unwrap();
    assert_eq!(reconcile(&state, first_attempt + 10).await, 1);
    assert!(
        state
            .metadata
            .requests()
            .request_check_evaluation(&request_id, &request_head)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        state
            .metadata
            .requests()
            .request_auto_merge_intent(&request_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        scope_domain::requests::RequestAutoMergeIntentStatus::Fulfilled
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconciliation_stops_when_the_authorizing_member_loses_access() {
    let (state, _source, _remote, request_id, request_head, _server) =
        native_open_request("request-auto-merge-revoked", ViewId::private()).await;
    let member_id = "user_auto_merge_member";
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(
            member_id,
            "auto-merge-member",
            "member@example.com",
        ))
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.collaboration.members.push(test_repository_member(
                TEST_REPO_ID,
                member_id,
                RepositoryMemberPermissions::default(),
            ));
        })
        .await
        .unwrap();
    let revision = state
        .metadata
        .requests()
        .latest_request_revision(&request_id)
        .await
        .unwrap()
        .unwrap();
    state
        .metadata
        .requests()
        .forget_request_check_evaluations_for_tests(&request_id)
        .await
        .unwrap();
    crate::use_cases::request_auto_merge::authorize(
        &state,
        &request_id,
        member_id,
        revision.id,
        request_head.clone(),
    )
    .await
    .unwrap();

    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.collaboration
                .members
                .retain(|member| member.user_id != member_id);
        })
        .await
        .unwrap();
    assert_eq!(reconcile(&state, unix_now()).await, 1);
    let intent = state
        .metadata
        .requests()
        .request_auto_merge_intent(&request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        intent.reason,
        Some(scope_domain::requests::RequestAutoMergeStopReason::AccessRevoked)
    );
    assert!(
        state
            .metadata
            .requests()
            .request_check_evaluation(&request_id, &request_head)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(live_file_content(&state, "/auto-merge.txt").await, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configuration_failure_stays_stopped_if_the_evaluation_later_clears() {
    let request_id = "req_auto_merge_failed_checks";
    let (state, revision_id) = open_owner_request(request_id).await;
    let request = state
        .metadata
        .requests()
        .request_for_tests(request_id)
        .await
        .unwrap()
        .unwrap();
    let revision = state
        .metadata
        .requests()
        .request_revision(request_id, &revision_id)
        .await
        .unwrap()
        .unwrap();
    let context = RequestCheckContext::for_revision(&request, &revision).unwrap();
    let app = router(state.clone());
    let active = authorize(app.clone(), request_id, &revision_id, SECOND_HEAD).await;
    assert_eq!(active["intent"]["status"], "Active");
    assert_eq!(active["waiting_reason"], "Waiting for CI evaluation");
    let attempt_time = unix_now() + 1;
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
                &context,
                "invalid request workflow",
                attempt_time,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    assert_eq!(reconcile(&state, attempt_time).await, 0);
    let stopped = auto_merge_json(app.clone(), "GET", request_id, None, StatusCode::OK).await;
    assert_stopped(&stopped, "ChecksConfigurationError");

    state
        .metadata
        .requests()
        .forget_request_check_evaluations_for_tests(request_id)
        .await
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
            evaluation: RequestCheckEvaluation::no_checks(&context, attempt_time + 1).unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    assert_eq!(
        state
            .metadata
            .requests()
            .request_check_evaluation(request_id, SECOND_HEAD)
            .await
            .unwrap()
            .unwrap()
            .state,
        scope_domain::requests::RequestCheckEvaluationState::NoChecks
    );
    assert_eq!(reconcile(&state, attempt_time + 2).await, 0);
    let still_stopped = auto_merge_json(app, "GET", request_id, None, StatusCode::OK).await;
    assert_stopped(&still_stopped, "ChecksConfigurationError");
}
