use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};

fn temp_cache_root(test: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "scope-{test}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn incarnation(repository_id: &str) -> RepositoryIncarnation {
    RepositoryIncarnation::new(repository_id, format!("repoi_{repository_id}"))
        .expect("test repository identity is valid")
}

#[test]
fn active_repository_is_not_evicted_when_the_cache_exceeds_its_byte_budget() {
    let root = temp_cache_root("git-cache-active");
    let registry = RepositoryGitCache::new(root.clone(), 50).unwrap();
    let active = incarnation("owner/active");
    let active_path = registry.path_for(&active);
    fs::create_dir_all(&active_path).unwrap();
    fs::write(active_path.join("pack"), [0_u8; 40]).unwrap();
    let lease = registry.lease(&active).unwrap();
    let inactive = incarnation("owner/inactive");
    let inactive_path = registry.path_for(&inactive);
    fs::create_dir_all(&inactive_path).unwrap();
    fs::write(inactive_path.join("pack"), [0_u8; 40]).unwrap();

    registry.prune().unwrap();

    assert!(active_path.exists());
    assert!(!inactive_path.exists());
    drop(lease);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn idle_repository_is_retained_while_the_cache_fits_its_byte_budget() {
    let root = temp_cache_root("git-cache-idle-retention");
    let registry = RepositoryGitCache::new(root.clone(), 1_000).unwrap();
    let repo = incarnation("owner/idle");
    let repo_path = registry.path_for(&repo);
    fs::create_dir_all(&repo_path).unwrap();
    fs::write(repo_path.join("pack"), [0_u8; 40]).unwrap();
    registry.note_applied(&repo, &repo_path, 1).unwrap();
    fs::File::options()
        .write(true)
        .open(repo_path.join(LAST_USED_FILE))
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
        .unwrap();

    registry.prune().unwrap();

    assert!(repo_path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn exact_repository_removal_waits_for_active_lease() {
    let root = temp_cache_root("git-cache-lease-safe-delete");
    let registry = RepositoryGitCache::new(root.clone(), 100).unwrap();
    let repo = incarnation("owner/recreated");
    let path = registry.path_for(&repo);
    fs::create_dir_all(&path).unwrap();
    let lease = registry.lease(&repo).unwrap();

    assert!(!registry.remove(&repo).unwrap());
    assert!(path.exists());
    drop(lease);
    assert!(registry.remove(&repo).unwrap());
    assert!(!path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn exact_repository_removal_waits_for_lease_from_another_registry() {
    let root = temp_cache_root("git-cache-cross-registry-lease-safe-delete");
    let leasing_registry = RepositoryGitCache::new(root.clone(), 100).unwrap();
    let deleting_registry = RepositoryGitCache::new(root.clone(), 100).unwrap();
    let repo = incarnation("owner/shared-root");
    let path = leasing_registry.path_for(&repo);
    fs::create_dir_all(&path).unwrap();
    let lease = leasing_registry.lease(&repo).unwrap();

    assert!(!deleting_registry.remove(&repo).unwrap());
    assert!(path.exists());
    drop(lease);
    assert!(deleting_registry.remove(&repo).unwrap());
    assert!(!path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn applied_frontier_requires_the_exact_repository_incarnation() {
    let root = temp_cache_root("git-cache-incarnation-marker");
    let registry = RepositoryGitCache::new(root.clone(), 100).unwrap();
    let predecessor = RepositoryIncarnation::new("owner/repo", "repoi_predecessor").unwrap();
    let recreated = RepositoryIncarnation::new("owner/repo", "repoi_recreated").unwrap();
    let recreated_path = registry.path_for(&recreated);
    fs::create_dir_all(&recreated_path).unwrap();
    registry
        .note_applied(&predecessor, &recreated_path, 41)
        .unwrap();

    assert_eq!(registry.applied_sequence(&recreated, &recreated_path), None);
    assert_ne!(registry.path_for(&predecessor), recreated_path);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stale_temporary_materialization_is_removed() {
    let root = temp_cache_root("git-cache-stale-materialization");
    fs::create_dir_all(&root).unwrap();
    let materialization = root.join("repo-materializing.tmp");
    fs::create_dir_all(&materialization).unwrap();

    prune_stale_materializations(&root, SystemTime::now(), Duration::ZERO).unwrap();

    assert!(!materialization.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn derived_repository_counts_toward_the_budget_and_is_leased_while_in_use() {
    let root = temp_cache_root("git-cache-derived");
    let registry = RepositoryGitCache::new(root.clone(), 50).unwrap();
    let derived_path = root.join("read-view-derived.git");
    fs::create_dir_all(&derived_path).unwrap();
    fs::write(derived_path.join("pack"), [0_u8; 40]).unwrap();
    let lease = registry.lease_derived(derived_path.clone()).unwrap();
    let inactive = incarnation("owner/inactive");
    let inactive_path = registry.path_for(&inactive);
    fs::create_dir_all(&inactive_path).unwrap();
    fs::write(inactive_path.join("pack"), [0_u8; 40]).unwrap();

    registry.prune().unwrap();

    assert!(derived_path.exists());
    assert!(!inactive_path.exists());
    drop(lease);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn least_recently_used_repository_is_evicted_when_cache_exceeds_byte_budget() {
    let root = temp_cache_root("git-cache-lru");
    let registry = RepositoryGitCache::new(root.clone(), 250).unwrap();
    let old = incarnation("owner/old");
    let old_path = registry.path_for(&old);
    fs::create_dir_all(&old_path).unwrap();
    fs::write(old_path.join("pack"), [0_u8; 40]).unwrap();
    registry.note_applied(&old, &old_path, 1).unwrap();
    thread::sleep(Duration::from_millis(10));

    let new = incarnation("owner/new");
    let new_path = registry.path_for(&new);
    fs::create_dir_all(&new_path).unwrap();
    fs::write(new_path.join("pack"), [0_u8; 40]).unwrap();
    registry.note_applied(&new, &new_path, 1).unwrap();

    assert!(old_path.exists());
    assert!(new_path.exists());

    registry.prune().unwrap();

    assert!(!old_path.exists());
    assert!(new_path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sanitizer_keeps_only_the_committed_main_ref() {
    let repo = std::env::temp_dir().join(format!(
        "scope-raw-cache-sanitize-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    run_git(
        None,
        &["init", repo.to_string_lossy().as_ref()],
        "init test repo",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &["config", "user.name", "Scope Test"],
        "set user",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &["config", "user.email", "scope@example.test"],
        "set email",
    )
    .unwrap();
    fs::write(repo.join("README.md"), "hello\n").unwrap();
    run_git(Some(&repo), &["add", "README.md"], "stage test file").unwrap();
    run_git(
        Some(&repo),
        &["commit", "-m", "initial"],
        "commit test file",
    )
    .unwrap();
    run_git(
        Some(&repo),
        &["branch", "-M", DEFAULT_GIT_BRANCH],
        "set default branch",
    )
    .unwrap();
    let head = String::from_utf8(
        run_git_output(Some(&repo), &["rev-parse", "HEAD"], "read test head")
            .unwrap()
            .stdout,
    )
    .unwrap();
    let head = head.trim();
    run_git(
        Some(&repo),
        &["update-ref", "refs/heads/private-request", head],
        "add request ref",
    )
    .unwrap();
    run_git(Some(&repo), &["tag", "private-tag"], "add tag").unwrap();

    sanitize_repository_git_cache_repo(&repo, head).unwrap();

    let output = run_git_output(
        Some(&repo),
        &["for-each-ref", "--format=%(refname)%00%(objectname)"],
        "read sanitized refs",
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("refs/heads/{DEFAULT_GIT_BRANCH}\0{head}\n")
    );
    let _ = fs::remove_dir_all(repo);
}

#[test]
fn concurrent_builds_coalesce_in_every_cache_namespace() {
    for namespace in [
        GitDerivedCacheNamespace::Projection,
        GitDerivedCacheNamespace::Repository,
        GitDerivedCacheNamespace::RequestReadView,
    ] {
        let coordinator = Arc::new(GitDerivedCacheCoordinator::default());
        let ready = Arc::new(AtomicBool::new(false));
        let builds = Arc::new(AtomicUsize::new(0));
        let (leader_started_tx, leader_started_rx) = mpsc::channel();
        let (release_leader_tx, release_leader_rx) = mpsc::channel();
        let leader = {
            let coordinator = coordinator.clone();
            let ready = ready.clone();
            let builds = builds.clone();
            thread::spawn(move || {
                coordinator.materialize(
                    namespace,
                    "same-key".to_string(),
                    || ready.load(Ordering::SeqCst),
                    || {
                        builds.fetch_add(1, Ordering::SeqCst);
                        leader_started_tx.send(()).unwrap();
                        release_leader_rx.recv().unwrap();
                        ready.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                )
            })
        };
        leader_started_rx.recv().unwrap();
        let follower = {
            let coordinator = coordinator.clone();
            let ready = ready.clone();
            thread::spawn(move || {
                coordinator.materialize(
                    namespace,
                    "same-key".to_string(),
                    || ready.load(Ordering::SeqCst),
                    || panic!("follower must not build"),
                )
            })
        };
        wait_for_follower(&coordinator, namespace, "same-key");
        release_leader_tx.send(()).unwrap();
        leader.join().unwrap().unwrap();
        follower.join().unwrap().unwrap();
        assert_eq!(builds.load(Ordering::SeqCst), 1, "{namespace:?}");
    }
}

#[test]
fn ready_cache_does_not_run_the_builder() {
    GitDerivedCacheCoordinator::default()
        .materialize(
            GitDerivedCacheNamespace::Repository,
            "ready-key".to_string(),
            || true,
            || panic!("ready cache must not build"),
        )
        .unwrap();
}

#[test]
fn externally_completed_cache_does_not_build_after_leader_election() {
    let readiness_checks = AtomicUsize::new(0);
    GitDerivedCacheCoordinator::default()
        .materialize(
            GitDerivedCacheNamespace::Repository,
            "externally-ready-key".to_string(),
            || readiness_checks.fetch_add(1, Ordering::SeqCst) > 0,
            || panic!("externally completed cache must not build"),
        )
        .unwrap();

    assert_eq!(readiness_checks.load(Ordering::SeqCst), 2);
}

#[test]
fn follower_with_a_newer_repository_frontier_runs_a_second_build() {
    let coordinator = Arc::new(GitDerivedCacheCoordinator::default());
    let first_ready = Arc::new(AtomicBool::new(false));
    let newer_ready = Arc::new(AtomicBool::new(false));
    let builds = Arc::new(AtomicUsize::new(0));
    let (leader_started_tx, leader_started_rx) = mpsc::channel();
    let (release_leader_tx, release_leader_rx) = mpsc::channel();
    let leader = {
        let coordinator = coordinator.clone();
        let first_ready = first_ready.clone();
        let builds = builds.clone();
        thread::spawn(move || {
            coordinator.materialize(
                GitDerivedCacheNamespace::Repository,
                "same-repository".to_string(),
                || first_ready.load(Ordering::SeqCst),
                || {
                    builds.fetch_add(1, Ordering::SeqCst);
                    leader_started_tx.send(()).unwrap();
                    release_leader_rx.recv().unwrap();
                    first_ready.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
        })
    };
    leader_started_rx.recv().unwrap();
    let follower = {
        let coordinator = coordinator.clone();
        let newer_ready = newer_ready.clone();
        let builds = builds.clone();
        thread::spawn(move || {
            coordinator.materialize(
                GitDerivedCacheNamespace::Repository,
                "same-repository".to_string(),
                || newer_ready.load(Ordering::SeqCst),
                || {
                    builds.fetch_add(1, Ordering::SeqCst);
                    newer_ready.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
        })
    };
    wait_for_follower(
        &coordinator,
        GitDerivedCacheNamespace::Repository,
        "same-repository",
    );
    release_leader_tx.send(()).unwrap();
    leader.join().unwrap().unwrap();
    follower.join().unwrap().unwrap();
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    assert!(newer_ready.load(Ordering::SeqCst));
}

#[test]
fn failed_build_is_shared_with_followers_and_then_can_retry() {
    let coordinator = Arc::new(GitDerivedCacheCoordinator::default());
    let builds = Arc::new(AtomicUsize::new(0));
    let (leader_started_tx, leader_started_rx) = mpsc::channel();
    let (release_leader_tx, release_leader_rx) = mpsc::channel();
    let leader = {
        let coordinator = coordinator.clone();
        let builds = builds.clone();
        thread::spawn(move || {
            coordinator.materialize(
                GitDerivedCacheNamespace::Projection,
                "failed-key".to_string(),
                || false,
                || {
                    builds.fetch_add(1, Ordering::SeqCst);
                    leader_started_tx.send(()).unwrap();
                    release_leader_rx.recv().unwrap();
                    Err(ApiError::infrastructure_unavailable("shared build failure"))
                },
            )
        })
    };
    leader_started_rx.recv().unwrap();
    let follower = {
        let coordinator = coordinator.clone();
        thread::spawn(move || {
            coordinator.materialize(
                GitDerivedCacheNamespace::Projection,
                "failed-key".to_string(),
                || false,
                || panic!("follower must not build"),
            )
        })
    };
    wait_for_follower(
        &coordinator,
        GitDerivedCacheNamespace::Projection,
        "failed-key",
    );
    release_leader_tx.send(()).unwrap();
    for worker in [leader, follower] {
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(error.kind, crate::error::ErrorKind::ServiceUnavailable);
        assert_eq!(error.operator_diagnostic(), "shared build failure");
    }
    assert_eq!(builds.load(Ordering::SeqCst), 1);

    let ready = AtomicBool::new(false);
    coordinator
        .materialize(
            GitDerivedCacheNamespace::Projection,
            "failed-key".to_string(),
            || ready.load(Ordering::SeqCst),
            || {
                builds.fetch_add(1, Ordering::SeqCst);
                ready.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(builds.load(Ordering::SeqCst), 2);
}

#[test]
fn cache_namespaces_do_not_hide_global_capacity_pressure() {
    let coordinator = Arc::new(GitDerivedCacheCoordinator::default());
    let budgets = Arc::new(crate::runtime_budgets::RuntimeBudgets::from_config(
        crate::runtime_budgets::RuntimeBudgetConfig {
            git_materialization_concurrency: 1,
            ..Default::default()
        },
    ));
    let projection_ready = Arc::new(AtomicBool::new(false));
    let (leader_started_tx, leader_started_rx) = mpsc::channel();
    let (release_leader_tx, release_leader_rx) = mpsc::channel();
    let leader = {
        let coordinator = coordinator.clone();
        let budgets = budgets.clone();
        let projection_ready = projection_ready.clone();
        thread::spawn(move || {
            coordinator.materialize(
                GitDerivedCacheNamespace::Projection,
                "shared-value".to_string(),
                || projection_ready.load(Ordering::SeqCst),
                || {
                    let _permit = budgets.try_git_materialization()?;
                    leader_started_tx.send(()).unwrap();
                    release_leader_rx.recv().unwrap();
                    projection_ready.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
        })
    };
    leader_started_rx.recv().unwrap();

    let error = coordinator
        .materialize(
            GitDerivedCacheNamespace::Repository,
            "shared-value".to_string(),
            || false,
            || {
                let _permit = budgets.try_git_materialization()?;
                Ok(())
            },
        )
        .unwrap_err();

    assert_eq!(error.kind, crate::error::ErrorKind::TooManyRequests);
    assert_eq!(
        error.operator_diagnostic(),
        "Git materialization capacity is exhausted; retry later"
    );
    release_leader_tx.send(()).unwrap();
    leader.join().unwrap().unwrap();
}

fn wait_for_follower(
    coordinator: &GitDerivedCacheCoordinator,
    namespace: GitDerivedCacheNamespace,
    key: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while coordinator.follower_count(namespace, key) == 0 {
        assert!(
            Instant::now() < deadline,
            "follower did not join the in-flight cache build"
        );
        thread::yield_now();
    }
}

#[test]
fn recursive_eviction_does_not_block_other_leases_or_delete_a_replacement() {
    let root = temp_cache_root("git-cache-eviction-lock");
    let registry = RepositoryGitCache::new(root.clone(), 100).unwrap();
    let repo = incarnation("owner/retired");
    let path = registry.path_for(&repo);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("old"), "old").unwrap();
    let deleting_path = path.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        evict_unleased(&deleting_path, |retired| {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            remove_dir_if_exists(retired)
        })
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (lease_tx, lease_rx) = mpsc::channel();
    let lease_registry = registry.clone();
    let lease_worker = thread::spawn(move || {
        let other = lease_registry.lease(&incarnation("owner/other")).unwrap();
        let replacement = lease_registry.lease(&repo).unwrap();
        fs::create_dir_all(replacement.as_ref()).unwrap();
        fs::write(replacement.join("new"), "new").unwrap();
        lease_tx.send((other, replacement)).unwrap();
    });
    let leases = lease_rx.recv_timeout(Duration::from_secs(2));
    release_tx.send(()).unwrap();
    assert!(worker.join().unwrap().unwrap());
    lease_worker.join().unwrap();
    let _leases = leases.expect("filesystem deletion held the cache registry lock");
    assert!(path.join("new").exists());
    assert!(!path.join("old").exists());
    fs::remove_dir_all(root).unwrap();
}
