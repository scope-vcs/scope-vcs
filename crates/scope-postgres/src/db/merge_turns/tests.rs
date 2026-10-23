use crate::db::{MetadataStore, TestDatabaseTarget};
use std::time::Duration;

#[tokio::test]
async fn merge_turns_queue_merges_of_one_repository_only() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let repositories = store.repositories();
    let first = repositories.acquire_merge_turn("owner/one").await.unwrap();
    let other = tokio::time::timeout(
        Duration::from_secs(5),
        repositories.acquire_merge_turn("owner/two"),
    )
    .await
    .expect("another repository's merge does not wait")
    .unwrap();
    let waiting_store = repositories.clone();
    let waiting =
        tokio::spawn(async move { waiting_store.acquire_merge_turn("owner/one").await.unwrap() });
    crate::db::locks::wait_for_advisory_waiter(&store, "merge-turn", "owner/one").await;
    assert!(
        !waiting.is_finished(),
        "a held merge turn admitted a second merge"
    );
    first.release().await;
    let second = tokio::time::timeout(Duration::from_secs(60), waiting)
        .await
        .unwrap()
        .unwrap();
    second.release().await;
    other.release().await;
}
