use super::*;
use crate::use_cases::request_ref_cleanup::drain_request_ref_cleanup_at;

async fn close_draft(state: &AppState, id: &str) -> Response {
    router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!("/v1/repos/{TEST_REPO_ID}/requests/{id}"))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn draft_deletion_keeps_success_and_durable_cleanup_after_git_failure() {
    let state = test_state_with_request().await;
    let (_source, _remote, _server, head) = request_checkout(&state, "draft-cleanup-failure").await;
    let store_repo =
        crate::git::storage::request_ref_store_repo_path(&state, &test_repo_incarnation());
    let lock = store_repo.join(format!("{REQUEST_REF}.lock"));
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    fs::write(&lock, "force update-ref failure").unwrap();
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let response = close_draft(&state, REQUEST_ID).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["deleted"], true);
    assert!(
        state
            .metadata
            .requests()
            .request_by_id(REQUEST_ID)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        events.try_recv().is_ok(),
        "committed deletion must publish its refresh"
    );
    assert_eq!(
        close_draft(&state, REQUEST_ID).await.status(),
        StatusCode::NOT_FOUND
    );

    let now = unix_now();
    let failed = drain_request_ref_cleanup_at(&state, now).await.unwrap();
    assert_eq!(failed.attempted, 1);
    assert_eq!(failed.failed.len(), 1);
    let pending = state
        .metadata
        .cleanup()
        .pending_request_ref_cleanups(None)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request_id, REQUEST_ID);
    assert_eq!(pending[0].head_oid, head);
    assert_eq!(pending[0].attempts, 1);
    assert!(pending[0].next_run_at_unix > now);
    assert!(pending[0].last_error.is_some());
    assert_eq!(
        git_stdout_text(
            &store_repo,
            &["rev-parse", REQUEST_REF],
            "read retained request ref"
        )
        .unwrap()
        .trim(),
        head
    );
    assert_eq!(
        drain_request_ref_cleanup_at(&state, now)
            .await
            .unwrap()
            .attempted,
        0
    );

    fs::remove_file(lock).unwrap();
    let retried = drain_request_ref_cleanup_at(&state, pending[0].next_run_at_unix)
        .await
        .unwrap();
    assert_eq!(retried.completed, 1);
    assert!(retried.failed.is_empty());
    assert!(
        state
            .metadata
            .cleanup()
            .pending_request_ref_cleanups(None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !run_git_output(
            Some(&store_repo),
            &["show-ref", "--verify", REQUEST_REF],
            "check deleted ref"
        )
        .unwrap()
        .status
        .success()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_draft_cleanup_preserves_a_same_name_replacement_even_at_the_same_head() {
    let state = test_state_with_request().await;
    let (_source, _remote, _server, head) =
        request_checkout(&state, "draft-cleanup-replacement").await;
    let mut replacement = stored_request(&state, REQUEST_ID).await;
    let response = close_draft(&state, REQUEST_ID).await;
    assert_eq!(response.status(), StatusCode::OK);
    replacement.id = "req_replacement".to_string();
    // Reusing the same Git head is deliberate: an expected-OID check alone
    // cannot distinguish this replacement from the deleted draft.
    state
        .metadata
        .requests()
        .insert_request_for_tests(replacement)
        .await
        .unwrap();
    assert!(
        state
            .metadata
            .cleanup()
            .request_ref_is_live(&test_repo_incarnation(), REQUEST_NAME)
            .await
            .unwrap()
    );
    let prior_incarnation =
        scope_domain::repository::RepositoryIncarnation::new(TEST_REPO_ID, "repoi_prior").unwrap();
    assert!(
        !state
            .metadata
            .cleanup()
            .request_ref_is_live(&prior_incarnation, REQUEST_NAME)
            .await
            .unwrap()
    );
    let report = drain_request_ref_cleanup_at(&state, unix_now())
        .await
        .unwrap();
    assert_eq!(report.completed, 1);
    assert!(report.failed.is_empty());
    let store_repo =
        crate::git::storage::request_ref_store_repo_path(&state, &test_repo_incarnation());
    assert_eq!(
        git_stdout_text(
            &store_repo,
            &["rev-parse", REQUEST_REF],
            "read replacement ref"
        )
        .unwrap()
        .trim(),
        head
    );
    assert_eq!(
        stored_request(&state, "req_replacement").await.head_oid,
        head
    );
    assert!(
        state
            .metadata
            .cleanup()
            .pending_request_ref_cleanups(None)
            .await
            .unwrap()
            .is_empty()
    );
}
