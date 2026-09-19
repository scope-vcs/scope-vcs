use super::*;
use crate::db::{
    SubmitRequestCommand,
    generated_ids::test_generated_id,
    requests::tests::{postgres_store, start_public_request},
};
use scope_domain::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    requests::{RecordRequestRevisionInput, RequestAutoMergeIntentStatus},
};

#[tokio::test]
async fn latest_same_second_intent_and_expired_claims_remain_fenced() {
    let store = postgres_store();
    let revision = open_request_with_revision(&store).await;
    let requests = store.requests();

    requests
        .authorize_request_auto_merge(authorize("intent_one", "enabled_one", &revision, 7))
        .await
        .unwrap();
    let first_claim = requests
        .claim_due_request_auto_merges(
            ClaimDueRequestAutoMergesCommand {
                now_unix: 7,
                lease_expires_at_unix: 10,
                limit: 1,
                request_id: Some("req_1".into()),
            },
            &test_generated_id,
        )
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(first_claim.attempt, 1);

    // The expired token cannot stop the intent after another executor reclaims it.
    let reclaimed = requests
        .claim_due_request_auto_merges(
            ClaimDueRequestAutoMergesCommand {
                now_unix: 11,
                lease_expires_at_unix: 20,
                limit: 1,
                request_id: Some("req_1".into()),
            },
            &test_generated_id,
        )
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(reclaimed.attempt, 2);
    assert_ne!(first_claim.claim_token, reclaimed.claim_token);
    assert!(
        requests
            .stop_claimed_request_auto_merge(StopClaimedRequestAutoMergeCommand {
                intent_id: "intent_one".into(),
                claim_token: first_claim.claim_token,
                reason: RequestAutoMergeStopReason::MergeConflict,
                event_id: "stale_stop".into(),
                now_unix: 11,
            })
            .await
            .unwrap()
            .is_none()
    );

    requests
        .cancel_request_auto_merge(CancelRequestAutoMergeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_owner".into(),
            expected_intent_id: "intent_one".into(),
            event_id: "cancel_one".into(),
            now_unix: 12,
        })
        .await
        .unwrap();
    assert!(
        !requests
            .release_request_auto_merge_claim(ReleaseRequestAutoMergeClaimCommand {
                intent_id: "intent_one".into(),
                claim_token: reclaimed.claim_token,
                next_attempt_at_unix: 13,
                last_error: Some("late executor".into()),
                now_unix: 12,
            })
            .await
            .unwrap()
    );

    // Re-enabling at the exact cancellation timestamp still reads the new intent by
    // request activity position, independent of random ids or timestamp resolution.
    requests
        .authorize_request_auto_merge(authorize("intent_two", "enabled_two", &revision, 12))
        .await
        .unwrap();
    let latest = requests
        .request_auto_merge_intent("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.id, "intent_two");
    assert_eq!(latest.status, RequestAutoMergeIntentStatus::Active);
}

async fn open_request_with_revision(store: &super::super::MetadataStore) -> String {
    start_public_request(store).await;
    store
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            event_id: "submitted".into(),
            now_unix: 4,
        })
        .await
        .unwrap();
    store
        .requests()
        .record_request_revision(
            RecordRequestRevisionInput {
                request_id: "req_1".into(),
                actor_user_id: "user_public".into(),
                actor_can_edit: true,
                expected_old_head_oid: Some("head".into()),
                new_head_oid: "a".repeat(40),
                git_snapshot: SourceBlob {
                    content_ref: ContentRef::git_bundle_sha256("sha256-auto-merge"),
                    sha256: "sha256-auto-merge".into(),
                    git_oid: "a".repeat(40),
                    git_file_mode: DEFAULT_GIT_FILE_MODE.into(),
                    size_bytes: 1,
                },
                event_id: "revision_current".into(),
                body: None,
                now_unix: 5,
            },
            &test_generated_id,
        )
        .await
        .unwrap()
        .revision
        .id
}

fn authorize(
    intent_id: &str,
    event_id: &str,
    revision_id: &str,
    now_unix: u64,
) -> AuthorizeRequestAutoMergeCommand {
    AuthorizeRequestAutoMergeCommand {
        request_id: "req_1".into(),
        actor_user_id: "user_owner".into(),
        expected_revision_id: revision_id.into(),
        expected_head_oid: "a".repeat(40),
        intent_id: intent_id.into(),
        event_id: event_id.into(),
        now_unix,
    }
}
