use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconciliation_evaluates_a_saved_intent_after_restart_without_a_viewer() {
    let (state, _source, _remote, request_id, request_head, _server) = native_open_request(
        "request-auto-merge-recover-checks",
        RequestAudience::Private,
    )
    .await;
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

    // The next process only has the saved intent; no request read or HTTP call
    // participates in this pass.
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
        native_open_request("request-auto-merge-retry-checks", RequestAudience::Public).await;
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
    state
        .metadata
        .repositories()
        .delete_repository_workflow_catalog_for_tests(TEST_REPO_ID)
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
    assert_eq!(
        state.backfill_repository_workflow_catalogs().await.unwrap(),
        1
    );
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
        native_open_request("request-auto-merge-revoked", RequestAudience::Private).await;
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

    // Simulate a missed revocation notification. Reconciliation must still
    // validate the actor's current access before evaluating checks.
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
