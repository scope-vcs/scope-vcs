use super::*;

#[tokio::test]
async fn independent_request_writes_share_repository_guard_without_hydrating_history() {
    use sea_orm::{ConnectionTrait, TransactionTrait};
    use std::time::Duration;
    let store = postgres_store();
    start_public_request(&store).await;
    let mut second = public_start_input();
    second.id = "req_independent".to_string();
    second.name = "independent".to_string();
    second.event_id = "event_independent".to_string();
    store.requests().start_request(second).await.unwrap();

    let held = store.db.begin().await.unwrap();
    super::super::super::request_access::lock_request_repository(&held, "req_1", "user_public")
        .await
        .unwrap();
    held.execute_unprepared("LOCK TABLE scope_logical_commits, scope_file_changes, scope_live_files IN ACCESS EXCLUSIVE MODE")
        .await.unwrap();
    let edited = tokio::time::timeout(
        Duration::from_secs(60),
        crate::db::test_support::request_fixtures::identity_edited(
            &store.requests(),
            scope_domain::requests::EditRequestIdentityInput {
                request_id: "req_independent".to_string(),
                actor_user_id: "user_public".to_string(),
                event_id: "edit_independent".to_string(),
                title: Some("Independent progress".to_string()),
                description_markdown: Some("Still authorized".to_string()),
                expected_description_markdown: None,
                now_unix: 5,
            },
        ),
    )
    .await
    .expect("an unrelated request must not wait for the first request or history tables");
    assert_eq!(edited.request.title, "Independent progress");
    held.rollback().await.unwrap();
}
