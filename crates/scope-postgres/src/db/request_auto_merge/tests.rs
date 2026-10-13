use super::*;
use crate::db::{
    RecordRequestChecksCommand, SubmitRequestCommand, acquire_aggregate_lock,
    generated_ids::test_generated_id,
    locks::wait_for_transaction_waiter,
    requests::tests::{postgres_store, start_public_request},
    test_support::fixtures::source_blob,
};
use scope_domain::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    requests::{
        NativeRequestCheck, RecordRequestRevisionInput, RequestAutoMergeIntentStatus, RequestCheck,
        RequestCheckContext, RequestCheckEvaluation, RequestCheckEvaluationState,
        RequestRevisionGitFacts,
    },
    runs::{
        run::Run,
        source::{RunSource, RunTrigger},
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

#[tokio::test]
async fn recording_configuration_error_stops_matching_active_intent_immediately() {
    let store = postgres_store();
    let revision = open_request_with_revision(&store).await;
    let requests = store.requests();
    requests
        .authorize_request_auto_merge(authorize(
            "intent_config_error",
            "enabled_config_error",
            &revision,
            7,
        ))
        .await
        .unwrap();

    requests
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::configuration_error(
                &request_check_context(&store).await,
                "invalid request workflow",
                8,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();

    let stopped = requests
        .request_auto_merge_intent("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopped.status, RequestAutoMergeIntentStatus::Stopped);
    assert_eq!(
        stopped.reason,
        Some(RequestAutoMergeStopReason::ChecksConfigurationError)
    );
    assert_eq!(stopped.updated_at_unix, 8);
}

#[tokio::test]
async fn irrelevant_or_replayed_configuration_errors_do_not_stop_auto_merge() {
    let store = postgres_store();
    open_request_with_revision(&store).await;
    let requests = store.requests();
    requests
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::configuration_error(
                &request_check_context(&store).await,
                "invalid request workflow",
                7,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    assert!(
        requests
            .request_auto_merge_intent("req_1")
            .await
            .unwrap()
            .is_none()
    );

    let store = postgres_store();
    let revision = open_request_with_revision(&store).await;
    let requests = store.requests();
    requests
        .authorize_request_auto_merge(authorize("intent_current", "enabled_current", &revision, 7))
        .await
        .unwrap();
    let mut stale_context = request_check_context(&store).await;
    stale_context.head_oid = "b".repeat(40);
    requests
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::configuration_error(
                &stale_context,
                "stale-head workflow error",
                8,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap_err();
    let active = requests
        .request_auto_merge_intent("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active.status, RequestAutoMergeIntentStatus::Active);
    assert_eq!(active.reason, None);

    let store = postgres_store();
    let revision = open_request_with_revision(&store).await;
    let requests = store.requests();
    requests
        .authorize_request_auto_merge(authorize("intent_replay", "enabled_replay", &revision, 7))
        .await
        .unwrap();
    requests
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::no_checks(&request_check_context(&store).await, 8)
                .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    let replayed = requests
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::configuration_error(
                &request_check_context(&store).await,
                "later conflicting evaluation",
                9,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    assert_eq!(
        replayed.evaluation.state,
        RequestCheckEvaluationState::NoChecks
    );
    let active = requests
        .request_auto_merge_intent("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(active.status, RequestAutoMergeIntentStatus::Active);
    assert_eq!(active.reason, None);
}

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
            },
            &test_generated_id,
        )
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(first_claim.attempt, 1);

    let reclaimed = requests
        .claim_due_request_auto_merges(
            ClaimDueRequestAutoMergesCommand {
                now_unix: 11,
                lease_expires_at_unix: 20,
                limit: 1,
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

#[tokio::test]
async fn lease_bookkeeping_does_not_advance_the_domain_transition_clock() {
    let store = postgres_store();
    let revision = open_request_with_revision(&store).await;
    let requests = store.requests();

    requests
        .authorize_request_auto_merge(authorize("intent_clock", "enabled_clock", &revision, 7))
        .await
        .unwrap();
    let claim = requests
        .claim_due_request_auto_merges(
            ClaimDueRequestAutoMergesCommand {
                now_unix: 20,
                lease_expires_at_unix: 30,
                limit: 1,
            },
            &test_generated_id,
        )
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(claim.intent.updated_at_unix, 7);
    assert_eq!(
        requests
            .request_auto_merge_intent("req_1")
            .await
            .unwrap()
            .unwrap()
            .updated_at_unix,
        7
    );
    assert!(
        requests
            .release_request_auto_merge_claim(ReleaseRequestAutoMergeClaimCommand {
                intent_id: "intent_clock".into(),
                claim_token: claim.claim_token,
                next_attempt_at_unix: 21,
                last_error: Some("retry later".into()),
                now_unix: 21,
            })
            .await
            .unwrap()
    );
    assert_eq!(
        requests
            .request_auto_merge_intent("req_1")
            .await
            .unwrap()
            .unwrap()
            .updated_at_unix,
        7
    );

    let cancelled = requests
        .cancel_request_auto_merge(CancelRequestAutoMergeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_owner".into(),
            expected_intent_id: "intent_clock".into(),
            event_id: "cancel_clock".into(),
            now_unix: 8,
        })
        .await
        .unwrap();
    assert_eq!(cancelled.intent.updated_at_unix, 8);
    assert_eq!(
        cancelled.intent.status,
        RequestAutoMergeIntentStatus::Cancelled
    );
}

#[tokio::test]
async fn authorization_committing_first_fences_a_selected_retention_candidate() {
    let store = postgres_store();
    let revision = open_request_with_revision(&store).await;
    record_terminal_check_evidence(&store, true).await;

    let guard = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&guard, "request", "req_1")
        .await
        .unwrap();
    let guard_pid = backend_pid(&guard).await;

    let authorize_store = store.clone();
    let authorize_revision = revision.clone();
    let authorize_task = tokio::spawn(async move {
        authorize_store
            .requests()
            .authorize_request_auto_merge(authorize(
                "intent_retained",
                "enabled_retained",
                &authorize_revision,
                7,
            ))
            .await
    });
    wait_for_transaction_waiter(&store, guard_pid).await;

    let retention_store = store.clone();
    let retention_task = tokio::spawn(async move {
        retention_store
            .runs()
            .prune_terminal_runs(100, 100, 1, &test_generated_id)
            .await
    });
    wait_for_blocked_transaction_count(&store, guard_pid, 2).await;
    guard.commit().await.unwrap();

    authorize_task.await.unwrap().unwrap();
    assert_eq!(retention_task.await.unwrap().unwrap(), 0);
    assert!(
        store
            .runs()
            .run("run-check-evidence")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn retention_committing_first_makes_authorization_reject_missing_evidence() {
    let store = postgres_store();
    let revision = open_request_with_revision(&store).await;
    record_terminal_check_evidence(&store, true).await;

    let guard = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&guard, "request", "req_1")
        .await
        .unwrap();
    let guard_pid = backend_pid(&guard).await;

    let retention_store = store.clone();
    let retention_task = tokio::spawn(async move {
        retention_store
            .runs()
            .prune_terminal_runs(100, 100, 1, &test_generated_id)
            .await
    });
    wait_for_transaction_waiter(&store, guard_pid).await;

    let authorize_store = store.clone();
    let authorize_task = tokio::spawn(async move {
        authorize_store
            .requests()
            .authorize_request_auto_merge(authorize(
                "intent_missing",
                "enabled_missing",
                &revision,
                7,
            ))
            .await
    });
    wait_for_blocked_transaction_count(&store, guard_pid, 2).await;
    guard.commit().await.unwrap();

    assert_eq!(retention_task.await.unwrap().unwrap(), 1);
    let error = authorize_task.await.unwrap().unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert_eq!(error.message, "request CI evidence is no longer available");
    assert!(
        store
            .runs()
            .run("run-check-evidence")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .requests()
            .request_auto_merge_intent("req_1")
            .await
            .unwrap()
            .is_none()
    );
}

mod authorization;
mod fixtures;
use fixtures::*;
