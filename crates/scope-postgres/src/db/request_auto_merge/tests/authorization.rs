use super::*;

#[tokio::test]
async fn authorization_rejects_current_failed_checks_without_creating_an_intent() {
    let store = postgres_store();
    let revision = open_request_with_revision(&store).await;
    record_terminal_check_evidence(&store, false).await;
    let request = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    let events = store.requests().request_events_for_tests().await.unwrap();
    let error = store
        .requests()
        .authorize_request_auto_merge(authorize(
            "failed_intent",
            "failed_authorization",
            &revision,
            7,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert_eq!(
        store
            .requests()
            .request_by_id("req_1")
            .await
            .unwrap()
            .unwrap(),
        request
    );
    assert!(
        store
            .requests()
            .request_auto_merge_intent("req_1")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.requests().request_events_for_tests().await.unwrap(),
        events
    );
}
