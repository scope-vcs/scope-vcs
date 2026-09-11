use super::*;
use crate::{
    db::{
        MetadataStore,
        locks::wait_for_transaction_waiter,
        test_support::fixtures::{repository, source_blob, store_with_repositories, user},
    },
    error::PostgresErrorKind,
};
use scope_domain::{policy::Visibility, repository::collaboration::RepositoryMember};
use sea_orm::{ConnectionTrait, DatabaseBackend, PaginatorTrait, Statement};

fn fixture() -> (
    MetadataStore,
    ManualRunRequest,
    WorkflowRevision,
    SourceBlob,
) {
    let mut repository = repository(&user("owner", "owner"), "repo", Visibility::Private);
    repository.members.push(RepositoryMember {
        repo_id: "owner/repo".into(),
        user_id: "member".into(),
        permissions: Default::default(),
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    let store = store_with_repositories([repository]);
    let request = ManualRunRequest::new(
        "owner/repo".into(),
        "member".into(),
        "1".repeat(32),
        "a".repeat(40),
        "checks".into(),
    )
    .unwrap();
    let revision = scope_run_config::parse_workflow(
        "/.scope/runs/checks.yml",
        format!("name: Checks\non:\n  manual: true\ncontainer: {{ image: rust@sha256:{} }}\ntimeout: 10m\njobs:\n  checks:\n    steps:\n      - {{ name: Test, run: cargo test }}\n", "b".repeat(64)).as_bytes(),
    ).unwrap().into_revision("owner/repo").unwrap();
    let object = source_blob(&"a".repeat(40), &"c".repeat(64), 42);
    (store, request, revision, object)
}

async fn assert_no_enqueue_rows(store: &MetadataStore) {
    assert_eq!(
        entities::run::Entity::find()
            .count(store.db.as_ref())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        entities::run_job::Entity::find()
            .count(store.db.as_ref())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        entities::workflow_revision::Entity::find()
            .count(store.db.as_ref())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        entities::object_reference::Entity::find()
            .count(store.db.as_ref())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn uploaded_enqueue_rechecks_membership_after_waiting_for_repository_lock() {
    let (store, request, revision, object) = fixture();
    let held = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&held, "repository", request.repository_id())
        .await
        .unwrap();
    let pid = held
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pg_backend_pid() AS pid",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "pid")
        .unwrap();
    entities::repository_member::Entity::delete_by_id((
        "owner/repo".to_owned(),
        "member".to_owned(),
    ))
    .exec(&held)
    .await
    .unwrap();
    let task_store = store.clone();
    let pending = tokio::spawn(async move {
        task_store
            .runs()
            .enqueue_uploaded_manual_run(&request, object, revision, 10)
            .await
    });
    wait_for_transaction_waiter(&store, pid).await;
    held.commit().await.unwrap();
    assert_eq!(
        pending.await.unwrap().err().unwrap().kind,
        PostgresErrorKind::PermissionDenied
    );
    assert_no_enqueue_rows(&store).await;
}

#[tokio::test]
async fn uploaded_enqueue_that_wins_repository_lock_completes_before_real_revocation() {
    let (store, request, revision, object) = fixture();
    let held = store.db.begin().await.unwrap();
    held.execute(Statement::from_string(
        DatabaseBackend::Postgres,
        "LOCK TABLE scope_runs IN SHARE MODE",
    ))
    .await
    .unwrap();
    let pid = held
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pg_backend_pid() AS pid",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "pid")
        .unwrap();
    let writing_store = store.clone();
    let writing = tokio::spawn(async move {
        writing_store
            .runs()
            .enqueue_uploaded_manual_run(&request, object, revision, 10)
            .await
    });
    let command_pid = wait_for_transaction_waiter(&store, pid).await;
    let revoking_store = store.clone();
    let revoking = tokio::spawn(async move {
        revoking_store
            .repositories()
            .remove_repository_member(
                "owner",
                "repo",
                "owner",
                "member",
                20,
                &crate::db::generated_ids::test_generated_id,
            )
            .await
    });
    wait_for_transaction_waiter(&store, command_pid).await;
    held.commit().await.unwrap();
    let enqueued = tokio::time::timeout(std::time::Duration::from_secs(60), writing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let removed = tokio::time::timeout(std::time::Duration::from_secs(60), revoking)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(enqueued.inserted);
    assert_eq!(removed.value.user_id, "member");
    assert_eq!(
        store.runs().run(&enqueued.run.id).await.unwrap().unwrap(),
        enqueued.run
    );
}

#[tokio::test]
async fn uploaded_enqueue_replay_requires_current_membership_and_matching_request() {
    let (store, request, revision, object) = fixture();
    let first = store
        .runs()
        .enqueue_uploaded_manual_run(&request, object.clone(), revision.clone(), 10)
        .await
        .unwrap();
    assert!(first.inserted);
    let replay = store
        .runs()
        .enqueue_uploaded_manual_run(&request, object.clone(), revision.clone(), 11)
        .await
        .unwrap();
    assert!(!replay.inserted);
    assert_eq!(first.run, replay.run);
    let other_actor = ManualRunRequest::new(
        "owner/repo".into(),
        "owner".into(),
        "1".repeat(32),
        "a".repeat(40),
        "checks".into(),
    )
    .unwrap();
    assert_eq!(
        store
            .runs()
            .enqueue_uploaded_manual_run(&other_actor, object.clone(), revision.clone(), 20)
            .await
            .err()
            .unwrap()
            .kind,
        PostgresErrorKind::Conflict
    );
    let tx = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&tx, "repository", request.repository_id())
        .await
        .unwrap();
    entities::repository_member::Entity::delete_by_id((
        "owner/repo".to_owned(),
        "member".to_owned(),
    ))
    .exec(&tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        store
            .runs()
            .enqueue_uploaded_manual_run(&request, object, revision, 30)
            .await
            .err()
            .unwrap()
            .kind,
        PostgresErrorKind::PermissionDenied
    );
    assert_eq!(
        store.runs().run(&request.run_id()).await.unwrap().unwrap(),
        first.run
    );
}

#[tokio::test]
async fn uploaded_enqueue_rejects_mismatched_source_without_persisting() {
    let (store, request, revision, mut object) = fixture();
    object.git_oid = "d".repeat(40);
    assert_eq!(
        store
            .runs()
            .enqueue_uploaded_manual_run(&request, object, revision, 10)
            .await
            .err()
            .unwrap()
            .kind,
        PostgresErrorKind::InvalidInput
    );
    assert_no_enqueue_rows(&store).await;
}

#[tokio::test]
async fn uploaded_enqueue_rejects_mismatched_workflow_without_persisting() {
    let (store, _, revision, object) = fixture();
    let request = ManualRunRequest::new(
        "owner/repo".into(),
        "member".into(),
        "1".repeat(32),
        "a".repeat(40),
        "other".into(),
    )
    .unwrap();
    assert_eq!(
        store
            .runs()
            .enqueue_uploaded_manual_run(&request, object, revision, 10)
            .await
            .err()
            .unwrap()
            .kind,
        PostgresErrorKind::InvalidInput,
    );
    assert_no_enqueue_rows(&store).await;
}

#[tokio::test]
async fn uploaded_enqueue_replay_preserves_dispatch_and_terminal_execution() {
    let (store, request, revision, object) = fixture();
    let runs = store.runs();
    let first = runs
        .enqueue_uploaded_manual_run(&request, object.clone(), revision.clone(), 10)
        .await
        .unwrap();
    let token = "e".repeat(64);
    runs.dispatch_job(
        &first.run.id,
        "checks",
        "replay-attempt",
        &token,
        "runtime",
        12,
        100,
    )
    .await
    .unwrap();
    let dispatched = runs.run_detail(&first.run.id).await.unwrap().unwrap();
    let replay = runs
        .enqueue_uploaded_manual_run(&request, object.clone(), revision.clone(), 13)
        .await
        .unwrap();
    assert!(!replay.inserted);
    assert_eq!(replay.run, dispatched.run);
    assert_eq!(
        runs.run_detail(&first.run.id).await.unwrap().unwrap(),
        dispatched
    );

    runs.expire_attempt("replay-attempt", 100).await.unwrap();
    runs.request_run_cancellation("member", "owner/repo", &first.run.id, 101)
        .await
        .unwrap();
    let terminal = runs.run_detail(&first.run.id).await.unwrap().unwrap();
    assert!(terminal.run.state.is_terminal());
    let replay = runs
        .enqueue_uploaded_manual_run(&request, object, revision, 102)
        .await
        .unwrap();
    assert!(!replay.inserted);
    assert_eq!(replay.run, terminal.run);
    assert_eq!(
        runs.run_detail(&first.run.id).await.unwrap().unwrap(),
        terminal
    );
}
