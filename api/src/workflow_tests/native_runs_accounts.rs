use super::*;
use crate::{error::ErrorKind, use_cases::native_runs};

#[tokio::test]
async fn listing_and_unlisting_an_account_notifies_its_repositories() {
    let state = test_state_with_repo();
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let availability_changed = serde_json::json!({
        "RepositoryChanged": { "reason": "native-runs-changed" }
    });

    let added = native_runs::add_account(
        &state.metadata,
        &state.repo_events,
        TEST_REPO_OWNER,
        Some("design partner".to_string()),
    )
    .await
    .unwrap();
    assert_eq!(added.handle, TEST_REPO_OWNER);
    assert_eq!(added.account.user_id, test_owner_id());
    assert_eq!(added.account.note.as_deref(), Some("design partner"));
    assert_eq!(
        serde_json::to_value(events.try_recv().unwrap().kind).unwrap(),
        availability_changed
    );
    assert_eq!(
        native_runs::list_accounts(&state.metadata).await.unwrap(),
        vec![added]
    );

    let removed = native_runs::remove_account(&state.metadata, &state.repo_events, TEST_REPO_OWNER)
        .await
        .unwrap();
    assert!(removed.removed);
    assert_eq!(removed.user_id, test_owner_id());
    assert_eq!(
        serde_json::to_value(events.try_recv().unwrap().kind).unwrap(),
        availability_changed
    );
    assert!(
        native_runs::list_accounts(&state.metadata)
            .await
            .unwrap()
            .is_empty()
    );

    assert_eq!(
        native_runs::add_account(&state.metadata, &state.repo_events, "nobody", None)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
}
