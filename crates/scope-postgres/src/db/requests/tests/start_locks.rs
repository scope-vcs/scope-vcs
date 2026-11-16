use super::*;
use sea_orm::TransactionTrait;
use std::time::Duration;

fn second_start(id: &str) -> StartRequestInput {
    let mut input = public_start_input();
    input.id = id.to_string();
    input.name = id.replace('_', "-");
    input.event_id = format!("event_{id}");
    input
}

#[tokio::test]
async fn a_start_does_not_wait_for_a_revision_holding_the_repository() {
    let store = postgres_store();
    start_public_request(&store).await;
    let held = store.db.begin().await.unwrap();
    super::super::super::request_access::lock_request_repository(&held, "req_1", "user_public")
        .await
        .unwrap();

    let started = tokio::time::timeout(
        Duration::from_secs(5),
        store
            .requests()
            .start_request(second_start("req_alongside")),
    )
    .await
    .expect("a start must not wait for a revision of another request")
    .unwrap();

    assert_eq!(started.request.id, "req_alongside");
    held.rollback().await.unwrap();
}

#[tokio::test]
async fn a_start_waits_for_an_exclusive_repository_mutation() {
    let store = postgres_store();
    let held = store.db.begin().await.unwrap();
    super::super::super::acquire_aggregate_lock(&held, "repository", "owner/repo")
        .await
        .unwrap();
    let held_pid = super::super::super::locks::backend_pid(&held).await;

    let starting_store = store.clone();
    let starting = tokio::spawn(async move {
        starting_store
            .requests()
            .start_request(public_start_input())
            .await
    });
    super::super::super::locks::wait_for_transaction_waiter(&store, held_pid).await;
    assert!(!starting.is_finished());
    held.commit().await.unwrap();

    assert_eq!(starting.await.unwrap().unwrap().request.id, "req_1");
}

#[tokio::test]
async fn a_start_that_waited_for_another_start_counts_its_draft() {
    let store = postgres_store();
    let first = store
        .requests()
        .start_request(public_start_input())
        .await
        .unwrap()
        .request;
    store
        .requests()
        .start_request(second_start("req_second"))
        .await
        .unwrap();
    let held = store.db.begin().await.unwrap();
    super::super::super::locks::acquire_shared_repository_lock(&held, "owner/repo")
        .await
        .unwrap();
    super::super::super::acquire_aggregate_lock(&held, "request-start", "owner/repo")
        .await
        .unwrap();
    let held_pid = super::super::super::locks::backend_pid(&held).await;
    let mut third = first;
    third.id = "req_third".into();
    third.name = "req-third".into();
    crate::db::request_rows::insert_request_row(&held, &third)
        .await
        .unwrap();

    let starting_store = store.clone();
    let starting = tokio::spawn(async move {
        starting_store
            .requests()
            .start_request(second_start("req_fourth"))
            .await
    });
    super::super::super::locks::wait_for_transaction_waiter(&store, held_pid).await;
    assert!(!starting.is_finished());
    held.commit().await.unwrap();

    let error = starting.await.unwrap().unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert!(error.message.contains("cannot have more than"));
}
