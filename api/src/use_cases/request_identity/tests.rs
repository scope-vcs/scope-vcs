use super::persist_identity_edit;
use crate::use_cases::request_test_fixtures::{public_start_input, seeded};
use scope_domain::{requests::EditRequestIdentityInput, views::ViewId};

#[tokio::test]
async fn an_edit_waiting_for_a_membership_removal_is_denied_after_it_commits() {
    let state = seeded(true);
    let store = &state.metadata;
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.view = ViewId::private();
    store.requests().start_request(start).await.unwrap();
    let removal = store
        .admin()
        .hold_repository_member_removal_for_tests("owner/repo", "user_public")
        .await
        .unwrap();
    let requests = store.requests();
    let editing = tokio::spawn(async move {
        persist_identity_edit(
            &requests,
            EditRequestIdentityInput {
                request_id: "req_1".into(),
                actor_user_id: "user_public".into(),
                event_id: "revoked_edit".into(),
                title: Some("Must not change".into()),
                description_markdown: Some(String::new()),
                expected_description_markdown: None,
                now_unix: 5,
            },
        )
        .await
    });
    removal.wait_for_waiter().await.unwrap();
    removal.commit().await.unwrap();

    assert!(editing.await.unwrap().is_err());
    let request = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(request.title, "Must not change");
}
