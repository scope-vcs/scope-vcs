use super::*;
use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime},
};

fn test_engine(test: &str) -> Arc<RepositoryEngine> {
    let root = std::env::temp_dir().join(format!(
        "scope-{test}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    RepositoryEngine::new(root, 1024 * 1024 * 1024).unwrap()
}

fn incarnation(repository_id: &str) -> RepositoryIncarnation {
    RepositoryIncarnation::new(repository_id, format!("repoi_{repository_id}"))
        .expect("test repository identity is valid")
}

#[test]
fn same_repository_operations_are_serialized() {
    let engine = test_engine("repository-engine-serial");
    let root = engine.cache_root().to_path_buf();
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let first_ready = Arc::new(AtomicBool::new(false));
    let second_ready = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let repo = incarnation("owner/repo");

    let first = {
        let engine = engine.clone();
        let active = active.clone();
        let max_active = max_active.clone();
        let first_ready = first_ready.clone();
        let repo = repo.clone();
        thread::spawn(move || {
            engine.coordinate_repository_blocking(
                &repo,
                || first_ready.load(Ordering::SeqCst),
                || {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(now, Ordering::SeqCst);
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                    first_ready.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
        })
    };
    started_rx.recv().unwrap();
    let second = {
        let engine = engine.clone();
        let active = active.clone();
        let max_active = max_active.clone();
        let second_ready = second_ready.clone();
        let repo = repo.clone();
        thread::spawn(move || {
            engine.coordinate_repository_blocking(
                &repo,
                || second_ready.load(Ordering::SeqCst),
                || {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(now, Ordering::SeqCst);
                    active.fetch_sub(1, Ordering::SeqCst);
                    second_ready.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
        })
    };
    thread::sleep(Duration::from_millis(20));
    assert_eq!(max_active.load(Ordering::SeqCst), 1);
    release_tx.send(()).unwrap();
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    assert_eq!(max_active.load(Ordering::SeqCst), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn independent_repositories_run_in_parallel() {
    let engine = test_engine("repository-engine-parallel");
    let root = engine.cache_root().to_path_buf();
    let (first_started_tx, first_started_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let first_repo = incarnation("owner/one");
    let first = {
        let engine = engine.clone();
        let first_repo = first_repo.clone();
        thread::spawn(move || {
            engine.coordinate_repository_blocking(
                &first_repo,
                || false,
                || {
                    first_started_tx.send(()).unwrap();
                    release_first_rx.recv().unwrap();
                    Ok(())
                },
            )
        })
    };
    first_started_rx.recv().unwrap();

    let (second_done_tx, second_done_rx) = mpsc::channel();
    let second = {
        let engine = engine.clone();
        let second_repo = incarnation("owner/two");
        thread::spawn(move || {
            engine.coordinate_repository_blocking(
                &second_repo,
                || false,
                || {
                    second_done_tx.send(()).unwrap();
                    Ok(())
                },
            )
        })
    };
    second_done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("independent repository was blocked");
    release_first_tx.send(()).unwrap();
    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    fs::remove_dir_all(root).unwrap();
}

struct IncrementalPushFixture {
    first_head: String,
    second_head: String,
    first_pack: PathBuf,
    second_pack: PathBuf,
}

fn incremental_push_fixture(engine: &RepositoryEngine) -> IncrementalPushFixture {
    let root = engine.cache_root();
    let work = root.join("work");
    let first_pack = root.join("first.pack");
    let second_pack = root.join("second.pack");
    run_git(
        None,
        &["init", work.to_string_lossy().as_ref()],
        "init test repo",
    )
    .unwrap();
    run_git(
        Some(&work),
        &["config", "user.name", "Scope Test"],
        "set user",
    )
    .unwrap();
    run_git(
        Some(&work),
        &["config", "user.email", "scope@example.test"],
        "set email",
    )
    .unwrap();
    fs::write(work.join("README.md"), "first\n").unwrap();
    run_git(Some(&work), &["add", "README.md"], "stage first commit").unwrap();
    run_git(
        Some(&work),
        &["commit", "-m", "first"],
        "create first commit",
    )
    .unwrap();
    run_git(
        Some(&work),
        &["branch", "-M", DEFAULT_GIT_BRANCH],
        "set main branch",
    )
    .unwrap();
    let first_head = git_head(&work);
    write_revision_pack(&work, &first_pack, format!("{first_head}\n"));

    fs::write(work.join("README.md"), "second\n").unwrap();
    run_git(
        Some(&work),
        &["commit", "-am", "second"],
        "create second commit",
    )
    .unwrap();
    let second_head = git_head(&work);
    write_revision_pack(
        &work,
        &second_pack,
        format!("{second_head}\n^{first_head}\n"),
    );
    IncrementalPushFixture {
        first_head,
        second_head,
        first_pack,
        second_pack,
    }
}

fn write_revision_pack(repo: &Path, destination: &Path, revisions: String) {
    let output = run_with_stdin_reader(
        Command::new("git")
            .current_dir(repo)
            .args(["pack-objects", "--revs", "--stdout"]),
        Cursor::new(revisions.into_bytes()),
        ProcessLimits::new(crate::runtime_budgets::RuntimeBudgets::default_git_command_timeout()),
        "creating test Git pack",
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(destination, output.stdout).unwrap();
}

#[test]
fn missing_cache_is_not_seeded_from_an_incremental_push() {
    let engine = test_engine("repository-engine-missing-incremental-base");
    let root = engine.cache_root().to_path_buf();
    let fixture = incremental_push_fixture(&engine);
    let repo = incarnation("owner/repo");
    let error = engine
        .sync_after_push(&repo, &fixture.second_pack, &fixture.second_head, 2)
        .unwrap_err();

    let replica = engine.repository_path(&repo);
    assert!(!replica.exists());
    assert_eq!(engine.cache.applied_sequence(&repo, &replica), None);
    assert!(
        error
            .operator_diagnostic()
            .contains("incremental Git segment cannot seed a missing repository cache")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn delayed_post_push_sync_cannot_regress_a_newer_replica() {
    let engine = test_engine("repository-engine-monotonic-sync");
    let root = engine.cache_root().to_path_buf();
    let fixture = incremental_push_fixture(&engine);
    let repo = incarnation("owner/repo");

    engine
        .sync_after_push(&repo, &fixture.first_pack, &fixture.first_head, 1)
        .unwrap();
    engine
        .sync_after_push(&repo, &fixture.second_pack, &fixture.second_head, 2)
        .unwrap();
    engine
        .sync_after_push(&repo, &fixture.first_pack, &fixture.first_head, 1)
        .unwrap();

    let replica = engine.repository_path(&repo);
    assert_eq!(git_head(&replica), fixture.second_head);
    assert_eq!(engine.cache.applied_sequence(&repo, &replica), Some(2));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn separate_engines_do_not_reuse_a_predecessor_incarnation_cache() {
    let predecessor_engine = test_engine("repository-engine-recreated-multi-engine");
    let root = predecessor_engine.cache_root().to_path_buf();
    let recreated_engine = RepositoryEngine::new(root.clone(), 1024 * 1024 * 1024).unwrap();
    let fixture = incremental_push_fixture(&predecessor_engine);
    let predecessor = RepositoryIncarnation::new("owner/repo", "repoi_predecessor").unwrap();
    let recreated = RepositoryIncarnation::new("owner/repo", "repoi_recreated").unwrap();

    predecessor_engine
        .sync_after_push(&predecessor, &fixture.first_pack, &fixture.first_head, 1)
        .unwrap();
    recreated_engine
        .sync_after_push(&recreated, &fixture.first_pack, &fixture.first_head, 1)
        .unwrap();

    let predecessor_path = predecessor_engine.repository_path(&predecessor);
    let recreated_path = recreated_engine.repository_path(&recreated);
    assert_ne!(predecessor_path, recreated_path);
    assert!(predecessor_path.exists());
    assert!(recreated_path.exists());
    assert_eq!(
        recreated_engine
            .cache
            .applied_sequence(&recreated, &recreated_path),
        Some(1)
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cancelled_derived_build_and_active_readers_remain_leased_during_eviction() {
    let root = tempfile::tempdir().unwrap();
    let engine = RepositoryEngine::new(root.path().to_path_buf(), 1).unwrap();
    let incarnation = RepositoryIncarnation::new("owner/repo", "repoi_derived").unwrap();
    let path = root.path().join("derived.git");
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let waiter = {
        let engine = engine.clone();
        let path = path.clone();
        let build_path = path.clone();
        let started = started.clone();
        let release = release.clone();
        let incarnation = incarnation.clone();
        tokio::spawn(async move {
            engine
                .materialize_derived(
                    &incarnation,
                    GitDerivedCacheNamespace::RequestRevision,
                    "derived".into(),
                    &path,
                    || false,
                    move || async move {
                        fs::create_dir(&build_path).unwrap();
                        fs::write(build_path.join("objects"), [0_u8; 32]).unwrap();
                        started.notify_one();
                        release.notified().await;
                        Ok(())
                    },
                )
                .await
        })
    };
    started.notified().await;
    waiter.abort();
    let _ = waiter.await;
    engine.cache.prune().unwrap();
    assert!(path.exists(), "detached build retains its own lease");
    let reader = engine.lease_derived(path.clone()).unwrap();
    release.notify_one();
    engine.cache.prune().unwrap();
    assert!(
        path.exists(),
        "reader retains the published materialization"
    );
    drop(reader);
}

fn git_head(repo: &Path) -> String {
    String::from_utf8(
        run_git_output(
            Some(repo),
            &["rev-parse", &format!("refs/heads/{DEFAULT_GIT_BRANCH}")],
            "read test head",
        )
        .unwrap()
        .stdout,
    )
    .unwrap()
    .trim()
    .to_string()
}
