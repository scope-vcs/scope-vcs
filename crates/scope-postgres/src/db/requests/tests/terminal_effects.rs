use super::*;
use scope_domain::requests::{GitHubBranch, GitHubPushDestination};

#[tokio::test]
async fn rolling_back_terminal_persistence_restores_request_invitees_push_and_event() {
    let store = postgres_store();
    let mut request = store
        .requests()
        .start_request(public_start_input())
        .await
        .unwrap()
        .request;
    request.submitted_at_unix = Some(3);
    request.updated_at_unix = 3;
    save_request_row(store.db.as_ref(), &request).await.unwrap();
    seed_terminal_effects(&store).await;

    let mutation = close_request(
        request,
        store
            .requests()
            .request_events_by_request_id("req_1")
            .await
            .unwrap(),
        Vec::new(),
        CloseRequestInput {
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            actor_is_maintainer: false,
            event_id: "event_rolled_back_close".into(),
            now_unix: 4,
        },
    )
    .unwrap();
    let CloseRequestMutation::Closed { request, event } = mutation else {
        panic!("submitted request should close");
    };
    let tx = store.db.begin().await.unwrap();
    persist_lifecycle_mutation(&tx, &request, std::slice::from_ref(&event))
        .await
        .unwrap();
    tx.rollback().await.unwrap();

    assert_eq!(
        store
            .requests()
            .request_for_tests("req_1")
            .await
            .unwrap()
            .unwrap()
            .state(),
        RequestState::Open
    );
    assert!(
        store
            .requests()
            .request_is_invitee("req_1", "user_guest")
            .await
            .unwrap()
    );
    assert!(
        store
            .requests()
            .latest_github_push("req_1")
            .await
            .unwrap()
            .unwrap()
            .target_oid
            .is_some()
    );
    assert!(
        store
            .requests()
            .request_events_by_request_id("req_1")
            .await
            .unwrap()
            .iter()
            .all(|event| event.id != "event_rolled_back_close")
    );
}

pub(crate) async fn seed_terminal_effects(store: &MetadataStore) {
    store
        .requests()
        .add_request_invitee(crate::db::AddRequestInviteeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            target_handle: "guest".into(),
            now_unix: 4,
        })
        .await
        .unwrap();
    crate::db::github_pushes::queue_github_push(
        store.db.as_ref(),
        "owner/repo",
        &GitHubBranch::Request("req_1".into()),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        &GitHubPushDestination {
            installation_id: 7,
            github_repository_id: 42,
            github_full_name: "octo/repo".into(),
        },
        4,
    )
    .await
    .unwrap();
}

pub(crate) async fn assert_terminal_effects(store: &MetadataStore, event_id: &str) {
    assert!(
        !store
            .requests()
            .request_is_invitee("req_1", "user_guest")
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .requests()
            .latest_github_push("req_1")
            .await
            .unwrap()
            .unwrap()
            .target_oid,
        None
    );
    assert!(
        store
            .requests()
            .request_events_by_request_id("req_1")
            .await
            .unwrap()
            .iter()
            .any(|event| event.id == event_id)
    );
}
