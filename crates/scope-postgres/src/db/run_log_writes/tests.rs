use super::*;
use crate::db::{
    CatalogFixture, MetadataStore, TestDatabaseTarget, locks::wait_for_transaction_waiter,
};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    policy::Visibility,
    repository::{RepoLifecycleState, Repository},
    runs::{
        run::Run,
        source::{RunSource, RunTrigger},
    },
};

async fn fixture() -> MetadataStore {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let owner = UserAccount {
        id: "owner".into(),
        handle: "owner".into(),
        email: "owner@scope.test".into(),
        email_verified: true,
    };
    let mut repo = Repository::new(&owner, "repo", Visibility::Private, "repoi_logs").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    let revision = scope_run_config::parse_workflow("/.scope/runs/checks.yml", format!(
        "name: Checks\non:\n  manual: true\ncontainer: {{ image: rust@sha256:{} }}\ntimeout: 10m\njobs:\n  a:\n    steps:\n      - {{ name: A, run: echo a }}\n  b:\n    steps:\n      - {{ name: B, run: echo b }}\n", "a".repeat(64)).as_bytes())
        .unwrap().into_revision("owner/repo").unwrap();
    let source = RunSource::ephemeral_git_bundle(SourceBlob {
        content_ref: ContentRef::git_bundle_sha256("c".repeat(64)),
        sha256: "c".repeat(64),
        git_oid: "d".repeat(40),
        git_file_mode: "100644".into(),
        size_bytes: 42,
    })
    .unwrap();
    let run = Run::new(
        "run",
        "run",
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some("owner".into()),
        source,
        10,
    )
    .unwrap();
    let runs = store.runs();
    runs.enqueue_run(run, revision).await.unwrap();
    for (job, token) in [("a", "a".repeat(64)), ("b", "b".repeat(64))] {
        runs.dispatch_job(
            "run",
            job,
            &format!("attempt-{job}"),
            &token,
            "runtime",
            11,
            100,
        )
        .await
        .unwrap();
        runs.start_attempt_step(&format!("attempt-{job}"), &token, 0, 12)
            .await
            .unwrap();
    }
    store
}

#[tokio::test]
async fn parallel_job_appends_publish_positions_in_commit_order() {
    let store = fixture().await;
    // Pause A after identity allocation, keeping its insertion uncommitted.
    store
        .db
        .execute_unprepared(
            r#"
        CREATE FUNCTION pause_first_log() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
            IF NEW.attempt_id = 'attempt-a' THEN
                PERFORM pg_advisory_xact_lock(731244910);
            END IF;
            RETURN NEW;
        END $$;
        CREATE TRIGGER pause_first_log AFTER INSERT ON scope_run_logs
            FOR EACH ROW EXECUTE FUNCTION pause_first_log();
    "#,
        )
        .await
        .unwrap();
    let held = store.db.begin().await.unwrap();
    let held_pid = held
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pg_advisory_xact_lock(731244910), pg_backend_pid() AS pid",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "pid")
        .unwrap();
    let first_store = store.clone();
    let first = tokio::spawn(async move {
        first_store
            .runs()
            .append_attempt_log(
                RunLogChunk::new("attempt-a", 0, 1, "first", 13).unwrap(),
                &"a".repeat(64),
                13,
            )
            .await
    });
    let first_pid = wait_for_transaction_waiter(&store, held_pid).await;
    let second_store = store.clone();
    let second = tokio::spawn(async move {
        second_store
            .runs()
            .append_attempt_log(
                RunLogChunk::new("attempt-b", 0, 1, "second", 13).unwrap(),
                &"b".repeat(64),
                13,
            )
            .await
    });
    // B must wait for A before allocating its position. Without the per-run lock
    // it commits now and a reader advances beyond A's still-invisible position.
    wait_for_transaction_waiter(&store, first_pid).await;
    assert!(
        store
            .runs()
            .run_logs_after("run", 0, 64)
            .await
            .unwrap()
            .is_empty()
    );
    held.commit().await.unwrap();
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert!(first.log.position < second.log.position);
    let first_page = store.runs().run_logs_after("run", 0, 1).await.unwrap();
    assert_eq!(first_page, vec![first.log.clone()]);
    let resumed = store
        .runs()
        .run_logs_after("run", first.log.position, 1)
        .await
        .unwrap();
    assert_eq!(resumed, vec![second.log.clone()]);
    assert!(
        store
            .runs()
            .run_logs_after("run", second.log.position, 1)
            .await
            .unwrap()
            .is_empty()
    );
}
