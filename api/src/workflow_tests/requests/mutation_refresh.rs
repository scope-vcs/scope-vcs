use super::*;
use crate::repo_events::{RepoChangeEvent, RepoChangeKind};
use std::time::Duration;
use tokio_stream::StreamExt;

#[tokio::test]
async fn committed_submission_refreshes_other_viewers_when_response_projection_fails() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    create_owner_request(&state, "req_failed_response", REQUEST_HEAD).await;
    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_failed_response", |request| {
            request.head_oid = "invalid-oid".to_string();
        })
        .await
        .unwrap();

    let events = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/events",
        None,
        None,
    )
    .await;
    assert_eq!(events.status(), StatusCode::OK);
    let mut stream = events.into_body().into_data_stream();
    let connected = stream.next().await.unwrap().unwrap();
    assert!(
        String::from_utf8(connected.to_vec())
            .unwrap()
            .contains("Connected")
    );

    let response = api_request(
        router(state.clone()),
        "POST",
        "/v1/repos/owner/repo/requests/req_failed_response/submit",
        Some(&bearer_header()),
        Some("{}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let committed = state
        .metadata
        .requests()
        .request_by_id("req_failed_response")
        .await
        .unwrap()
        .unwrap();
    assert!(committed.submitted_at_unix.is_some());

    let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("other viewer should receive a refresh after the commit")
        .unwrap()
        .unwrap();
    let event = String::from_utf8(event.to_vec()).unwrap();
    assert!(event.contains("repo-changed"), "{event}");
    assert!(!event.contains("request-submitted"), "{event}");
}

#[tokio::test]
async fn request_mutations_refresh_once_after_commit_and_never_after_rejection() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    create_owner_request(&state, "req_refresh_once", REQUEST_HEAD).await;
    let app = router(state.clone());
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let submit_path = "/v1/repos/owner/repo/requests/req_refresh_once/submit";
    let start_path = "/v1/repos/owner/repo/requests";
    let identity_path = "/v1/repos/owner/repo/requests/req_refresh_once";
    let auth = bearer_header();

    let submitted = api_request(app.clone(), "POST", submit_path, Some(&auth), Some("{}")).await;
    assert_eq!(submitted.status(), StatusCode::OK);
    assert_one_refresh(&mut events, "request-submitted");
    let rejected = api_request(app.clone(), "POST", submit_path, Some(&auth), Some("{}")).await;
    assert_eq!(rejected.status(), StatusCode::CONFLICT);
    assert!(events.try_recv().is_err());

    let start_body = r#"{"name":"refresh-once","audience":"Private"}"#;
    let started = api_request(
        app.clone(),
        "POST",
        start_path,
        Some(&auth),
        Some(start_body),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    assert_one_refresh(&mut events, "request-started");
    let rejected = api_request(
        app.clone(),
        "POST",
        start_path,
        Some(&auth),
        Some(start_body),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::CONFLICT);
    assert!(events.try_recv().is_err());

    let identity_body = r#"{"title":"Updated title"}"#;
    let edited = api_request(
        app.clone(),
        "PATCH",
        identity_path,
        Some(&auth),
        Some(identity_body),
    )
    .await;
    assert_eq!(edited.status(), StatusCode::OK);
    assert_one_refresh(&mut events, "request-identity-edited");
    let rejected = api_request(
        app,
        "PATCH",
        identity_path,
        Some(&auth),
        Some(identity_body),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::CONFLICT);
    assert!(events.try_recv().is_err());
}

fn assert_one_refresh(
    events: &mut tokio::sync::broadcast::Receiver<RepoChangeEvent>,
    reason: &str,
) {
    let event = events
        .try_recv()
        .expect("committed mutation should publish a refresh");
    assert_eq!(event.version, 0);
    assert_eq!(event.repo_id, TEST_REPO_ID);
    assert_eq!(
        event.incarnation_id,
        test_repo_incarnation().incarnation_id()
    );
    assert_eq!(
        event.kind,
        RepoChangeKind::RepositoryChanged {
            reason: reason.to_string()
        }
    );
    assert!(
        events.try_recv().is_err(),
        "mutation published more than once"
    );
}
