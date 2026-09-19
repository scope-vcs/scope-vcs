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
        RecordRequestRevisionInput, RequestAutoMergeIntentStatus, RequestCheck,
        RequestCheckEvaluation,
    },
    runs::{
        run::Run,
        source::{RunSource, RunTrigger},
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

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

    // The expired token cannot stop the intent after another executor reclaims it.
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

    // This command captured its timestamp before waiting for the later lease write.
    // Lease bookkeeping must not make the valid domain transition appear to predate it.
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
    record_terminal_check_evidence(&store).await;

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
    record_terminal_check_evidence(&store).await;

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
    guard.commit().await.unwrap();

    assert_eq!(retention_task.await.unwrap().unwrap(), 1);
    let error = authorize_task.await.unwrap().unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert_eq!(
        error.message,
        "request check evidence is no longer available"
    );
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

async fn record_terminal_check_evidence(store: &super::super::MetadataStore) {
    let revision = scope_run_config::parse_workflow(
        "/.scope/runs/check.yml",
        br#"
name: Check
on:
  manual: true
container:
  image: alpine@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
timeout: 5m
jobs:
  check:
    steps:
      - name: Check
        run: echo ok
"#,
    )
    .unwrap()
    .into_revision("owner/repo")
    .unwrap();
    let run = Run::new(
        "run-check-evidence",
        "run-check-evidence",
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some("user_owner".into()),
        RunSource::ephemeral_git_bundle(source_blob(&"a".repeat(40), &"b".repeat(64), 1)).unwrap(),
        5,
    )
    .unwrap();
    store
        .runs()
        .enqueue_run(run, revision.clone())
        .await
        .unwrap();
    store
        .runs()
        .request_run_cancellation("user_owner", "owner/repo", "run-check-evidence", 6)
        .await
        .unwrap();

    let mut check = RequestCheck::for_revision(&revision);
    check.run_id = Some("run-check-evidence".into());
    store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            evaluation: RequestCheckEvaluation::started("req_1", "a".repeat(40), vec![check], 6)
                .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
        })
        .await
        .unwrap();
}

async fn backend_pid(tx: &sea_orm::DatabaseTransaction) -> i32 {
    tx.query_one(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT pg_backend_pid() AS pid",
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get("", "pid")
    .unwrap()
}

async fn wait_for_blocked_transaction_count(
    store: &super::super::MetadataStore,
    blocker_pid: i32,
    expected: i64,
) {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let count = store
                .db
                .query_one(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "WITH RECURSIVE blocked(pid) AS ( \
                         SELECT pid FROM pg_stat_activity \
                          WHERE $1 = ANY(pg_blocking_pids(pid)) \
                         UNION \
                         SELECT activity.pid FROM pg_stat_activity activity \
                           JOIN blocked ON blocked.pid = ANY(pg_blocking_pids(activity.pid)) \
                     ) SELECT count(*) AS count FROM blocked",
                    [blocker_pid.into()],
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get::<i64>("", "count")
                .unwrap();
            if count >= expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("expected transactions blocked by the request guard");
}
