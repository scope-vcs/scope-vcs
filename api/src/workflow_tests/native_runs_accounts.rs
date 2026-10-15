use super::*;
use crate::use_cases::native_runs;

#[tokio::test]
async fn listing_and_unlisting_an_account_notifies_its_repositories() {
    let state = test_state_with_repo();
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let availability_changed = serde_json::json!({
        "RepositoryChanged": { "reason": "native-runs-changed" }
    });

    native_runs::add_account(&state.metadata, &state.repo_events, TEST_REPO_OWNER, None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(events.try_recv().unwrap().kind).unwrap(),
        availability_changed
    );

    native_runs::remove_account(&state.metadata, &state.repo_events, TEST_REPO_OWNER)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(events.try_recv().unwrap().kind).unwrap(),
        availability_changed
    );
}
