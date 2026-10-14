use super::*;
use crate::job_trace::test_support::{linked, queued_by};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    policy::ScopePath,
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin},
    repository::{RepoLifecycleState, Repository},
    views::ViewId,
};
use std::sync::atomic::{AtomicU64, Ordering};

const NOW: u64 = 1_700_000_000;

#[test]
fn retry_backoff_is_bounded() {
    assert_eq!(retry_delay_seconds(1), 5);
    assert_eq!(retry_delay_seconds(2), 10);
    assert_eq!(retry_delay_seconds(7), 300);
    assert_eq!(retry_delay_seconds(99), 300);
}

#[test]
fn retry_policy_stops_at_max_attempts() {
    assert_eq!(next_retry_attempt(0).unwrap(), 1);
    assert!(!is_terminal_retry_attempt(MAX_JOB_ATTEMPTS - 1));
    assert!(is_terminal_retry_attempt(MAX_JOB_ATTEMPTS));
    assert!(is_terminal_retry_attempt(MAX_JOB_ATTEMPTS + 10));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_role_rebuilds_repository_history() {
    let Ok(admin_url) = std::env::var("SCOPE_WORKER_ROLE_TEST_ADMIN_URL") else {
        return;
    };
    let worker_url = std::env::var("SCOPE_WORKER_ROLE_TEST_WORKER_URL")
        .expect("worker URL must accompany the admin URL");
    let admin = Arc::new(sea_orm::Database::connect(admin_url).await.unwrap());
    let worker = Arc::new(sea_orm::Database::connect(worker_url).await.unwrap());
    let owner = UserAccount {
        id: "worker_history_owner".into(),
        handle: "worker_history_owner".into(),
        email: "worker-history@example.com".into(),
        email_verified: true,
    };
    let mut repo = Repository::new(
        &owner,
        "worker-history",
        ViewId::public(),
        "worker_history_repo",
    )
    .unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    repo.graph.commits.push(LogicalCommit {
        occurred_at_unix: None,
        id: "worker_history_commit".into(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: format!("{:040x}", 1),
        },
        author_id: owner.id.clone(),
        message: "Initial file".into(),
        changes: vec![FileChange {
            path: ScopePath::parse("/file.txt").unwrap(),
            old_content: None,
            new_content: Some(SourceBlob {
                content_ref: ContentRef::git_bundle_sha256("worker-history-bundle"),
                sha256: "worker-history-hash".into(),
                git_oid: format!("{:040x}", 1),
                git_file_mode: "100644".into(),
                size_bytes: 1,
            }),
            label: ViewId::public(),
        }],
    });
    let mut catalog = crate::db::CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog
        .repositories
        .insert(repo.record.id.clone(), repo.clone());
    super::super::AdminStore {
        db: Arc::clone(&admin),
    }
    .seed_catalog_for_tests(catalog)
    .unwrap();

    let summary = JobStore {
        db: Arc::clone(&worker),
    }
    .run_ready_outbox_jobs("role-test-worker", 1, &|| Ok(1_700_000_001))
    .await
    .unwrap();
    let job_error = admin
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT last_error FROM scope_outbox_jobs LIMIT 1",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<Option<String>>("", "last_error")
        .unwrap();
    assert_eq!(summary.completed, 1, "{summary:?}, error: {job_error:?}");
    assert_eq!(summary.failed, 0);
    let entry_count = history_entry_count(admin.as_ref()).await;
    assert!(
        entry_count > 0,
        "outbox rebuild must insert history entries"
    );

    for sql in [
        "INSERT INTO scope_repository_history_payloads (repo_id, payload_hash, payload) VALUES ($1, 'stale', '{}'::jsonb)",
        "INSERT INTO scope_repository_history_entries (repo_id, audience, position, source_id, payload_hash) VALUES ($1, 'private', 1000000, 'stale-entry', 'stale')",
    ] {
        admin
            .execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                sql,
                [repo.record.id.clone().into()],
            ))
            .await
            .unwrap();
    }
    assert_eq!(history_entry_count(admin.as_ref()).await, entry_count + 1);
    super::super::projection_read_models::reset_live_projection_read_models(
        worker.as_ref(),
        &repo.record.id,
    )
    .await
    .unwrap();
    fold_live_projection_read_models(
        worker.as_ref(),
        &repo.record.id,
        repo.record.content_version,
    )
    .await
    .unwrap();
    let replaced_count = history_entry_count(admin.as_ref()).await;
    assert_eq!(
        replaced_count, entry_count,
        "a rebuild from the start must replace old entries"
    );
}

async fn history_entry_count(db: &sea_orm::DatabaseConnection) -> i64 {
    db.query_one_raw(sea_orm::Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT count(*) AS count FROM scope_repository_history_entries",
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<i64>("", "count")
    .unwrap()
}

#[tokio::test]
async fn batch_refreshes_time_before_each_claim_and_completion() {
    const START: u64 = 1_700_000_000;
    const STEP: u64 = 100;

    let target = super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    let (repo_id, repo_version) = seed_outbox_repo(&store).await;
    enqueue_projection_read_model_rebuild(
        store.db.as_ref(),
        &repo_id,
        repo_version.saturating_add(1),
        START,
    )
    .await
    .unwrap();

    let elapsed = AtomicU64::new(0);
    let clock = || Ok(START + elapsed.fetch_add(STEP, Ordering::SeqCst));
    let summary = store
        .jobs()
        .run_ready_outbox_jobs("worker", 2, &clock)
        .await
        .unwrap();

    assert_eq!(summary.claimed, 2);
    assert_eq!(summary.completed, 2);
    assert_eq!(summary.failed, 0);
    assert_eq!(elapsed.load(Ordering::SeqCst), 4 * STEP);

    let completed_at = entities::outbox_job::Entity::find()
        .order_by_asc(entities::outbox_job::Column::CompletedAtUnix)
        .all(store.db.as_ref())
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.completed_at_unix.unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        completed_at,
        vec![
            i64::try_from(START + STEP).unwrap(),
            i64::try_from(START + 3 * STEP).unwrap(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repository_deletion_makes_claimed_outbox_job_obsolete() {
    let target = super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    seed_outbox_repo(&store).await;
    let incarnation = store
        .repositories()
        .repository("owner", "repo")
        .await
        .unwrap()
        .unwrap()
        .incarnation();

    let (claimed_tx, claimed_rx) = tokio::sync::oneshot::channel();
    let (deleted_tx, deleted_rx) = tokio::sync::oneshot::channel();
    let worker_store = store.clone();
    let worker = tokio::spawn(async move {
        let job = claim(&worker_store, "worker")
            .await?
            .expect("the rebuild job should be ready");
        claimed_tx.send(job.id.clone()).unwrap();
        deleted_rx.await.unwrap();
        execute_outbox_job(worker_store.db.as_ref(), &job, NOW).await?;
        complete_outbox_job(worker_store.db.as_ref(), &job, "worker", NOW as i64).await
    });

    let job_id = claimed_rx.await.unwrap();
    store
        .repositories()
        .delete_repo("owner", "repo", &incarnation, "user_owner", NOW)
        .await
        .unwrap();
    assert!(
        entities::outbox_job::Entity::find_by_id(job_id)
            .one(store.db.as_ref())
            .await
            .unwrap()
            .is_none(),
        "repository deletion should cascade the claimed outbox job"
    );
    deleted_tx.send(()).unwrap();

    worker.await.unwrap().unwrap();
}

#[tokio::test]
async fn live_repository_still_reports_a_stolen_outbox_lease() {
    let target = super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    seed_outbox_repo(&store).await;
    let job = claim(&store, "worker").await.unwrap().unwrap();
    entities::outbox_job::Entity::update_many()
        .filter(entities::outbox_job::Column::Id.eq(job.id.clone()))
        .col_expr(
            entities::outbox_job::Column::LeaseOwner,
            Expr::value(Some("other-worker".to_string())),
        )
        .exec(store.db.as_ref())
        .await
        .unwrap();

    let error = complete_outbox_job(store.db.as_ref(), &job, "worker", NOW as i64)
        .await
        .unwrap_err();
    assert!(error.message.contains("lease was lost"));
    assert!(
        store
            .repositories()
            .repository("owner", "repo")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn claims_are_exclusive_and_expired_leases_are_reclaimed() {
    let target = super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    seed_outbox_repo(&store).await;
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut tasks = tokio::task::JoinSet::new();
    for worker in ["one", "two"] {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            barrier.wait().await;
            (worker, claim(&store, worker).await.unwrap())
        });
    }
    let mut claims = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let (worker, claim) = result.unwrap();
        if let Some(claim) = claim {
            claims.push((worker, claim));
        }
    }
    assert_eq!(claims.len(), 1);
    let (stale_owner, stale) = claims.pop().unwrap();

    entities::outbox_job::Entity::update_many()
        .filter(entities::outbox_job::Column::Id.eq(stale.id.clone()))
        .col_expr(
            entities::outbox_job::Column::LeaseExpiresAtUnix,
            Expr::value(Some(0_i64)),
        )
        .exec(store.db.as_ref())
        .await
        .unwrap();
    let current = claim(&store, "replacement").await.unwrap().unwrap();
    assert_eq!(current.id, stale.id);
    assert!(
        complete_outbox_job(store.db.as_ref(), &stale, stale_owner, NOW as i64)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn terminal_retry_marks_job_failed() {
    let target = super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    seed_outbox_repo(&store).await;
    let mut job = claim(&store, "worker").await.unwrap().unwrap();
    job.attempts = MAX_JOB_ATTEMPTS - 1;

    fail_outbox_job(
        store.db.as_ref(),
        &job,
        "worker",
        "failed".to_string(),
        NOW as i64,
    )
    .await
    .unwrap();
    let row = entities::outbox_job::Entity::find_by_id(job.id)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.state, JOB_FAILED);
    assert_eq!(row.attempts, MAX_JOB_ATTEMPTS);
    assert!(row.completed_at_unix.is_some());
}

#[tokio::test]
async fn a_claimed_job_links_the_trace_that_queued_it() {
    let target = super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    let (repo_id, repo_version) = seed_outbox_repo(&store).await;
    queued_by(
        "trace-outbox-push",
        enqueue_projection_read_model_rebuild(
            store.db.as_ref(),
            &repo_id,
            repo_version.saturating_add(1),
            NOW,
        ),
    )
    .await
    .unwrap();

    let summary = store
        .jobs()
        .run_ready_outbox_jobs("worker-a", 2, &|| Ok(NOW))
        .await
        .unwrap();

    assert_eq!(summary.completed, 2);
    assert!(linked("trace-outbox-push"));
}

async fn claim(
    store: &MetadataStore,
    worker: &str,
) -> Result<Option<ClaimedOutboxJob>, PostgresError> {
    claim_next_ready_job(store.db.as_ref(), worker, 60, NOW as i64).await
}

async fn seed_outbox_repo(store: &MetadataStore) -> (String, u64) {
    let owner = UserAccount {
        id: "user_owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    };
    let mut catalog = crate::db::CatalogFixture::default();
    let repo = catalog
        .create_repository(&owner, "repo", ViewId::private())
        .unwrap()
        .clone();
    catalog.users.insert(owner.id.clone(), owner);
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    enqueue_projection_read_model_rebuild(
        store.db.as_ref(),
        &repo.record.id,
        repo.record.change_version,
        1_700_000_000,
    )
    .await
    .unwrap();
    (repo.record.id, repo.record.change_version)
}
