use super::{CloseRequestCommand, persist_close};
use crate::use_cases::request_test_fixtures::{seeded, start_public_request};
use scope_domain::requests::{CloseRequestMutation, RequestState};

#[tokio::test]
async fn a_maintainer_closing_an_open_request_is_recorded_as_its_closer() {
    let state = seeded(false);
    let store = &state.metadata;
    start_public_request(store).await;
    store
        .requests()
        .mutate_request_for_tests("req_1", |request| {
            request.submitted_at_unix = Some(4);
            request.updated_at_unix = 4;
        })
        .await
        .unwrap();

    let mutation = persist_close(
        &store.requests(),
        CloseRequestCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_owner".into(),
            event_id: "event_closed".into(),
            now_unix: 6,
        },
    )
    .await
    .unwrap();

    assert!(matches!(mutation, CloseRequestMutation::Closed { .. }));
    let stored = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state(), RequestState::Closed);
    assert_eq!(stored.closed_at_unix, Some(6));
    assert_eq!(stored.closed_by_user_id.as_deref(), Some("user_owner"));
    assert!(
        store
            .requests()
            .request_events_by_request_id("req_1")
            .await
            .unwrap()
            .iter()
            .any(|event| event.id == "event_closed")
    );
}
