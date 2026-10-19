use super::*;
use sea_orm::TransactionTrait;
use std::time::Duration;

async fn store_with_independent_request() -> MetadataStore {
    let store = postgres_store();
    start_public_request(&store).await;
    let mut second = public_start_input();
    second.id = "req_independent".to_string();
    second.name = "independent".to_string();
    second.event_id = "event_independent".to_string();
    store.requests().start_request(second).await.unwrap();
    store
}

fn independent_revision(new_head: &str) -> RecordRequestRevisionInput {
    RecordRequestRevisionInput {
        new_base: None,
        new_base_stack_depth: None,
        request_id: "req_independent".to_string(),
        actor_user_id: "user_public".to_string(),
        actor_can_edit: true,
        expected_old_head_oid: Some("base".to_string()),
        new_head_oid: new_head.to_string(),
        git_snapshot: source_blob(new_head),
        git_facts: RequestRevisionGitFacts {
            contains_old_head: true,
            contained_main_oid: None,
            contained_main_descends_from_base: false,
        },
        event_id: format!("revision_{new_head}"),
        body: None,
        now_unix: 5,
    }
}

#[tokio::test]
async fn revisions_of_independent_requests_do_not_wait_for_each_other() {
    let store = store_with_independent_request().await;
    let held = store.db.begin().await.unwrap();
    super::super::super::request_access::lock_request_repository(&held, "req_1", "user_public")
        .await
        .unwrap();

    let revised = tokio::time::timeout(
        Duration::from_secs(5),
        store
            .requests()
            .record_request_revision(independent_revision("independent_head")),
    )
    .await
    .expect("a revision must not wait for another request in the same repository")
    .unwrap();

    assert_eq!(revised.request.head_oid, "independent_head");
    held.rollback().await.unwrap();
}

#[tokio::test]
async fn revisions_wait_for_an_exclusive_repository_mutation() {
    let store = store_with_independent_request().await;
    let held = store.db.begin().await.unwrap();
    super::super::super::acquire_aggregate_lock(&held, "repository", "owner/repo")
        .await
        .unwrap();
    let held_pid = super::super::super::locks::backend_pid(&held).await;

    let revising_store = store.clone();
    let revising = tokio::spawn(async move {
        revising_store
            .requests()
            .record_request_revision(independent_revision("independent_head"))
            .await
    });
    super::super::super::locks::wait_for_transaction_waiter(&store, held_pid).await;
    assert!(!revising.is_finished());
    held.commit().await.unwrap();

    let revised = revising.await.unwrap().unwrap();
    assert_eq!(revised.request.head_oid, "independent_head");
}
