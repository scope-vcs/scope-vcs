use super::persist_submission;
use crate::use_cases::request_test_fixtures::{public_start_input, public_upload_input, seeded};
use scope_domain::{requests::SubmitRequestInput, views::ViewId};
use scope_postgres::error::PostgresErrorKind;

#[tokio::test]
async fn a_submission_waiting_for_a_membership_removal_is_denied_and_appends_nothing() {
    let state = seeded(true);
    let store = &state.metadata;
    let mut start = public_start_input();
    start.view = ViewId::private();
    store.requests().start_request(start).await.unwrap();
    store
        .requests()
        .record_working_request_upload(public_upload_input())
        .await
        .unwrap();
    let draft = store.requests().request_by_id("req_1").await.unwrap();
    let events = store
        .requests()
        .request_events_by_request_id("req_1")
        .await
        .unwrap();
    let removal = store
        .admin()
        .hold_repository_member_removal_for_tests("owner/repo", "user_public")
        .await
        .unwrap();
    let requests = store.requests();
    let submitting = tokio::spawn(async move {
        persist_submission(
            &requests,
            SubmitRequestInput {
                request_id: "req_1".into(),
                actor_user_id: "user_public".into(),
                event_id: "revoked_submission".into(),
                now_unix: 5,
            },
        )
        .await
    });
    removal.wait_for_waiter().await.unwrap();
    removal.commit().await.unwrap();

    let error = submitting.await.unwrap().unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::PermissionDenied);
    assert_eq!(error.message, "request submission access required");
    assert_eq!(
        store.requests().request_by_id("req_1").await.unwrap(),
        draft
    );
    assert_eq!(
        store
            .requests()
            .request_events_by_request_id("req_1")
            .await
            .unwrap(),
        events
    );
}
