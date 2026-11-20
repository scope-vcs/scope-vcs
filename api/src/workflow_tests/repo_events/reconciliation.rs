use super::*;
use crate::after_commit::AfterCommit;
use crate::auth::cli::CliAuthService;
use scope_domain::requests::EditRequestIdentityInput;
use scope_domain::views::ViewId;

async fn advance_reconciliation() {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(30)).await;
    tokio::time::resume();
}

fn synced_through(event: &str) -> u64 {
    let (_, through) = event
        .split_once(r#""through_unix_ms":"#)
        .unwrap_or_else(|| panic!("expected a Synced event, got {event}"));
    through
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap()
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
async fn a_reader_that_lost_notifications_ends_its_streams_and_resyncs_reconnects() {
    let writer = test_state_with_repo();
    cache_test_jwks(&writer);
    let reader = AppState {
        repo_events: Default::default(),
        ..writer.clone()
    };
    let request_id = "request_missed_notification";
    writer
        .metadata
        .requests()
        .start_request(StartRequestInput {
            base: scope_domain::requests::RequestBase::Main {
                review_base_oid: ("a".repeat(40)).clone(),
            },
            id: request_id.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: "missed-notification".to_string(),
            author_user_id: test_owner_id(),
            title: Some("Before".to_string()),
            author_role: RequestActorRole::Owner,
            author_view: scope_domain::views::ViewId::private(),
            view: ViewId::private(),
            main_ancestor_oid: "a".repeat(40),
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
    let synced_through = synced_through(&next_event(&mut stream).await);

    crate::use_cases::request_identity::persist_identity_edit(
        &writer.metadata.requests(),
        EditRequestIdentityInput {
            request_id: request_id.to_string(),
            actor_user_id: test_owner_id(),
            event_id: "event_request_title_after_commit".to_string(),
            title: Some("After".to_string()),
            description_markdown: None,
            expected_description_markdown: None,
            now_unix: unix_now(),
        },
    )
    .await
    .unwrap();
    writer.after_commit([AfterCommit::request_summary(
        &test_repo_incarnation(),
        RepoChangeReason::RequestIdentityEdited,
    )]);
    assert_eq!(
        find_repo(&reader, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .record
            .change_version,
        version,
        "repository version polling alone cannot discover request mutations"
    );

    reader.repo_events.notifications_lost();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .unwrap()
            .is_none()
    );
    let reconnected = events_since(reader.clone(), Some(bearer_header()), synced_through).await;
    let event = next_event(&mut reconnected.into_body().into_data_stream()).await;
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
async fn a_reconnect_replays_the_changes_it_missed_or_resyncs_when_they_are_gone() {
    let state = AppState {
        repo_events: crate::repo_events::RepoChangeBus::complete_after(0),
        ..test_state_with_repo()
    };
    cache_test_jwks(&state);
    let response = events(state.clone(), Some(bearer_header())).await;
    let since = synced_through(&next_event(&mut response.into_body().into_data_stream()).await);
    state.after_commit([AfterCommit::request_summary(
        &test_repo_incarnation(),
        RepoChangeReason::RequestIdentityEdited,
    )]);

    let replayed = events_since(state.clone(), Some(bearer_header()), since).await;
    let mut replayed = replayed.into_body().into_data_stream();
    let missed = next_event(&mut replayed).await;
    assert!(missed.contains("request-identity-edited"), "{missed}");
    let caught_up = next_event(&mut replayed).await;
    assert!(synced_through(&caught_up) >= since, "{caught_up}");

    let stale = events_since(state, Some(bearer_header()), 1).await;
    let mut stale = stale.into_body().into_data_stream();
    let event = next_event(&mut stale).await;
    assert!(event.contains(r#""kind":"Lagged""#), "{event}");
    assert!(next_event(&mut stale).await.contains(r#""kind":{"Synced""#));
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
            .contains(r#""kind":{"Synced""#)
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
            .contains(r#""kind":{"Synced""#)
    );

    advance_reconciliation().await;
    let event = next_application_event(&mut stream).await;
    assert!(event.contains(r#""kind":{"Synced""#), "{event}");
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
            .contains(r#""kind":{"Synced""#)
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
