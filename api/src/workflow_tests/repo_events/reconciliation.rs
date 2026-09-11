use super::*;
use crate::auth::cli::CliAuthService;
use scope_postgres::db::EditRequestIdentityCommand;

async fn advance_reconciliation() {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(30)).await;
    tokio::time::resume();
}

async fn next_application_event(stream: &mut axum::body::BodyDataStream) -> String {
    loop {
        let event = next_event(stream).await;
        if !event.starts_with(':') {
            return event;
        }
    }
}

#[tokio::test]
async fn a_reader_reconciles_a_committed_request_change_without_its_notification() {
    let writer = test_state_with_repo();
    cache_test_jwks(&writer);
    let reader = AppState {
        // A second API instance whose LISTEN connection did not receive the write.
        repo_events: Default::default(),
        ..writer.clone()
    };
    let request_id = "request_missed_notification";
    writer
        .metadata
        .requests()
        .start_request(StartRequestInput {
            id: request_id.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: "missed-notification".to_string(),
            author_user_id: test_owner_id(),
            title: Some("Before".to_string()),
            author_role: RequestActorRole::Owner,
            audience: RequestAudience::Private,
            base_main_oid: "a".repeat(40),
            event_id: "event_request_missed_notification".to_string(),
            now_unix: unix_now(),
        })
        .await
        .unwrap();
    let version = find_repo(&reader, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .record
        .change_version;
    let response = events(reader.clone(), Some(bearer_header())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    assert!(
        next_event(&mut stream)
            .await
            .contains(r#""kind":"Connected""#)
    );

    writer
        .metadata
        .requests()
        .edit_request_identity(EditRequestIdentityCommand {
            request_id: request_id.to_string(),
            actor_user_id: test_owner_id(),
            event_id: "event_request_title_after_commit".to_string(),
            title: Some("After".to_string()),
            description_markdown: None,
            expected_description_markdown: None,
            now_unix: unix_now(),
        })
        .await
        .unwrap();
    writer
        .publish_request_summary_refresh(
            &test_repo_incarnation(),
            RepoChangeReason::RequestIdentityEdited,
        )
        .await;
    assert_eq!(
        find_repo(&reader, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .record
            .change_version,
        version,
        "repository version polling alone cannot discover request mutations"
    );

    advance_reconciliation().await;
    let event = next_application_event(&mut stream).await;
    assert!(event.contains(r#""kind":"Lagged""#), "{event}");
    let request = reader
        .metadata
        .requests()
        .request_by_id(request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.title, "After");
}

#[tokio::test]
async fn an_idle_stream_closes_after_its_cli_session_is_revoked() {
    let state = test_state_with_repo();
    let service = CliAuthService::new(state.metadata.auth());
    let user = test_user(test_owner_id(), TEST_REPO_OWNER, TEST_OWNER_EMAIL);
    let grant = service
        .create_exchange_grant(&user, unix_now())
        .await
        .unwrap();
    let session = service
        .exchange_grant(&grant.exchange_token, unix_now())
        .await
        .unwrap();
    let response = events(
        state.clone(),
        Some(format!("Bearer {}", session.session_token)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    assert!(
        next_event(&mut stream)
            .await
            .contains(r#""kind":"Connected""#)
    );

    service
        .revoke_session_token(&session.session_token, unix_now())
        .await
        .unwrap();
    advance_reconciliation().await;
    let event = next_application_event(&mut stream).await;
    assert!(event.contains("event: error"), "{event}");
    assert!(event.contains(r#""code":"unauthorized""#), "{event}");
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn anonymous_reconciliation_never_exposes_private_versions_or_event_details() {
    let state = test_state_with_readme().await;
    let response = events(state, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    assert!(
        next_event(&mut stream)
            .await
            .contains(r#""kind":"Connected""#)
    );

    advance_reconciliation().await;
    let event = next_application_event(&mut stream).await;
    assert!(event.contains(r#""kind":"Lagged""#), "{event}");
    assert!(event.contains(r#""version":0"#), "{event}");
    assert!(!event.contains("request_id"), "{event}");
    assert!(!event.contains("run_id"), "{event}");
}

#[tokio::test]
async fn reconciliation_closes_a_stream_after_an_unannounced_repository_recreation() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let response = events(state.clone(), Some(bearer_header())).await;
    let mut stream = response.into_body().into_data_stream();
    assert!(
        next_event(&mut stream)
            .await
            .contains(r#""kind":"Connected""#)
    );

    let mut recreated = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    recreated.record.incarnation_id = "repoi_reconciled_recreation".to_string();
    state
        .metadata
        .repositories()
        .recreate_repository_for_tests(recreated)
        .await
        .unwrap();
    advance_reconciliation().await;
    let event = next_application_event(&mut stream).await;
    assert!(event.contains("event: error"), "{event}");
    assert!(event.contains(r#""code":"conflict""#), "{event}");
    assert!(stream.next().await.is_none());
}
