use super::*;
use crate::git::{
    command::{run_git, run_git_output},
    import::git_push_from_repo,
};
use scope_domain::{
    account::UserAccount, policy::Visibility, projection::ProjectionViewKey,
    repository::git::GitHead, runs::source::RunSource,
};
use std::time::Instant;

#[tokio::test]
async fn concurrent_file_reads_and_run_bundles_reuse_objects_at_the_requested_revision() {
    let mut state = AppState::test_state();
    let owner = UserAccount {
        id: "user-owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.test".to_string(),
        email_verified: true,
    };
    let mut catalog = scope_postgres::db::CatalogFixture::default();
    catalog
        .create_repository(&owner, "repo", Visibility::Private)
        .unwrap();
    catalog.users.insert(owner.id.clone(), owner);
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(catalog)
        .unwrap();
    let repository = tempfile::tempdir().unwrap();
    run_git(
        None,
        &["init", "-b", "main", repository.path().to_str().unwrap()],
        "initialize source repo",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["config", "user.email", "scope@test.invalid"],
        "configure source repository email",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["config", "user.name", "Scope test"],
        "configure source repository name",
    )
    .unwrap();
    let content = (0_u64..32_768)
        .flat_map(|index| Sha256::digest(index.to_le_bytes()))
        .collect::<Vec<_>>();
    fs::write(repository.path().join("README.md"), &content).unwrap();
    run_git(
        Some(repository.path()),
        &["add", "README.md"],
        "stage source file",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["commit", "-m", "pin source"],
        "commit source file",
    )
    .unwrap();

    let pushed = git_push_from_repo(&state, "owner/repo", repository.path(), None)
        .await
        .unwrap();
    let source = RunSource::accepted_git_head(
        "owner/repo",
        GitHead {
            change_version: 1,
            ..pushed.stored.head.clone()
        },
        vec![pushed.stored.pack_span.clone()],
        ProjectionViewKey::Private,
    )
    .unwrap();

    state
        .git_segment_store
        .cleanup_local("owner/repo", &pushed.stored.pack_span.segment.segment_id)
        .await
        .unwrap();
    let incarnation = state
        .metadata
        .repositories()
        .git_push_context("owner", "repo", "user-owner")
        .await
        .unwrap()
        .unwrap()
        .incarnation;
    state.runtime_budgets =
        std::sync::Arc::new(crate::runtime_budgets::RuntimeBudgets::from_config(
            crate::runtime_budgets::RuntimeBudgetConfig {
                git_materialization_concurrency: 1,
                ..Default::default()
            },
        ));
    let blob_oid = String::from_utf8(
        run_git_output(
            Some(repository.path()),
            &["rev-parse", "HEAD:README.md"],
            "read file object ID",
        )
        .unwrap()
        .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let blob = scope_domain::content::SourceBlob {
        content_ref: scope_domain::content_ref::ContentRef::GitBlob {
            git_oid: blob_oid.clone(),
        },
        sha256: hex::encode(Sha256::digest(&content)),
        git_oid: blob_oid,
        git_file_mode: "100644".into(),
        size_bytes: content.len() as u64,
    };
    let spans = vec![pushed.stored.pack_span.clone()];
    for result in futures_util::future::join_all((0..8).map(|_| {
        crate::git::content::source_content_bytes(
            &state,
            &blob,
            Some((incarnation.clone(), &pushed.stored.head, &spans)),
        )
    }))
    .await
    {
        assert_eq!(result.unwrap(), content);
    }
    let retained = state
        .git_segment_store
        .get_verified_pack(&incarnation, &pushed.stored.pack_span.segment)
        .await
        .unwrap();
    let index_path = retained.path().with_extension("idx");
    let indexed_at = fs::metadata(&index_path).unwrap().modified().unwrap();
    state
        .git_segment_store
        .delete_remote(&scope_storage::segment_object_key(
            "owner/repo",
            &pushed.stored.pack_span.segment.segment_id,
        ))
        .await
        .unwrap();
    assert!(
        state
            .repository_engine
            .delete_repository_cache(&incarnation)
            .unwrap()
    );
    assert_eq!(
        crate::git::content::source_content_bytes(
            &state,
            &blob,
            Some((incarnation.clone(), &pushed.stored.head, &spans))
        )
        .await
        .unwrap(),
        content
    );
    assert_eq!(
        fs::metadata(index_path).unwrap().modified().unwrap(),
        indexed_at
    );
    drop(retained);
    // Advance the shared replica before bundling the earlier accepted revision.
    fs::write(repository.path().join("README.md"), "newer content").unwrap();
    run_git(
        Some(repository.path()),
        &["commit", "-am", "advance main"],
        "advance source",
    )
    .unwrap();
    let newer = git_push_from_repo(
        &state,
        "owner/repo",
        repository.path(),
        Some(&pushed.stored.head),
    )
    .await
    .unwrap();
    state
        .repository_engine
        .materialize_repository(
            &state,
            &incarnation,
            &newer.stored.head,
            &[
                pushed.stored.pack_span.clone(),
                newer.stored.pack_span.clone(),
            ],
        )
        .await
        .unwrap();
    state
        .git_segment_store
        .delete_remote(&scope_storage::segment_object_key(
            "owner/repo",
            &pushed.stored.pack_span.segment.segment_id,
        ))
        .await
        .unwrap();
    let started = Instant::now();
    let results = futures_util::future::join_all((0..8).map(|_| {
        materialize_accepted_git_head_bundle(&state, &incarnation, &source, 4 * 1024 * 1024)
    }))
    .await;
    let cold_elapsed = started.elapsed();
    let mut results = results.into_iter();
    let materialized = results.next().unwrap().unwrap();
    let materialized_sha256 = materialized.sha256.clone();
    let materialized_bytes = bundle_bytes(materialized).await;
    assert!(!materialized_bytes.is_empty());
    for concurrent in results {
        let concurrent = concurrent.unwrap();
        assert_eq!(concurrent.sha256, materialized_sha256);
        assert_eq!(bundle_bytes(concurrent).await, materialized_bytes);
    }
    assert_eq!(
        materialized_sha256,
        hex::encode(Sha256::digest(&materialized_bytes))
    );

    // Warm reads require neither Git admission nor the original remote pack.
    state
        .git_segment_store
        .cleanup_local("owner/repo", &pushed.stored.pack_span.segment.segment_id)
        .await
        .unwrap();
    state
        .git_segment_store
        .delete_remote(&scope_storage::segment_object_key(
            "owner/repo",
            &pushed.stored.pack_span.segment.segment_id,
        ))
        .await
        .unwrap();
    let _busy = state.runtime_budgets.try_git_materialization().unwrap();
    let started = Instant::now();
    let warm = materialize_accepted_git_head_bundle(&state, &incarnation, &source, 4 * 1024 * 1024)
        .await
        .unwrap();
    let warm_sha256 = warm.sha256.clone();
    let warm_bytes = bundle_bytes(warm).await;
    eprintln!(
        "run source eight cold followers: {:?}; warm read: {:?}; bundle bytes: {}",
        cold_elapsed,
        started.elapsed(),
        warm_bytes.len()
    );
    assert_eq!(warm_bytes, materialized_bytes);
    assert_eq!(warm_sha256, materialized_sha256);
    assert!(
        materialize_accepted_git_head_bundle(&state, &incarnation, &source, 1)
            .await
            .is_err()
    );
    let recreated = RepositoryIncarnation::new("owner/repo", "different-incarnation").unwrap();
    assert!(
        materialize_accepted_git_head_bundle(&state, &recreated, &source, 4 * 1024 * 1024)
            .await
            .is_err()
    );

    let bundle = repository.path().join("source.bundle");
    fs::write(&bundle, &materialized_bytes).unwrap();
    let output = std::process::Command::new("git")
        .args(["bundle", "list-heads", bundle.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(&pushed.stored.head.head_oid));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&newer.stored.head.head_oid));
    let restored = repository.path().join("pinned.git");
    run_git(
        None,
        &[
            "clone",
            "--bare",
            bundle.to_str().unwrap(),
            restored.to_str().unwrap(),
        ],
        "inspect pinned bundle",
    )
    .unwrap();
    assert_eq!(
        run_git_output(
            Some(&restored),
            &["show", "main:README.md"],
            "read pinned content"
        )
        .unwrap()
        .stdout,
        content
    );
}

async fn git_head_fixture(state: &AppState) -> (RunSource, TemporarySourceDirectory) {
    let owner = UserAccount {
        id: "user-owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.test".to_string(),
        email_verified: true,
    };
    let mut catalog = scope_postgres::db::CatalogFixture::default();
    catalog
        .create_repository(&owner, "repo", Visibility::Private)
        .unwrap();
    catalog.users.insert(owner.id.clone(), owner);
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(catalog)
        .unwrap();
    let repository = TemporarySourceDirectory::new(&state.data_dir.join("run-source")).unwrap();
    fs::create_dir_all(repository.path()).unwrap();
    run_git(
        None,
        &["init", "-b", "main", repository.path().to_str().unwrap()],
        "initialize source repo",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["config", "user.email", "scope@test.invalid"],
        "configure source repository email",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["config", "user.name", "Scope test"],
        "configure source repository name",
    )
    .unwrap();
    fs::write(repository.path().join("README.md"), "pinned run source").unwrap();
    run_git(
        Some(repository.path()),
        &["add", "README.md"],
        "stage source file",
    )
    .unwrap();
    run_git(
        Some(repository.path()),
        &["commit", "-m", "pin source"],
        "commit source file",
    )
    .unwrap();

    let pushed = git_push_from_repo(state, "owner/repo", repository.path(), None)
        .await
        .unwrap();
    let source = RunSource::accepted_git_head(
        "owner/repo",
        GitHead {
            change_version: 1,
            ..pushed.stored.head.clone()
        },
        vec![pushed.stored.pack_span.clone()],
        ProjectionViewKey::Private,
    )
    .unwrap();

    state
        .git_segment_store
        .cleanup_local("owner/repo", &pushed.stored.pack_span.segment.segment_id)
        .await
        .unwrap();
    (source, repository)
}

#[tokio::test]
async fn cancelled_revision_and_bundle_requests_keep_repository_and_capacity_until_exit() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    // Pause revision setup or bundle creation after shared hydration completes.
    for phase in [1, 2] {
        for outcome in ["success", "failure", "panic"] {
            let state = AppState::test_state();
            let (source, _fixture) = git_head_fixture(&state).await;
            let incarnation = state
                .metadata
                .repositories()
                .git_push_context("owner", "repo", "user-owner")
                .await
                .unwrap()
                .unwrap()
                .incarnation;
            let (_, head, spans) = source.logical_git_head().unwrap();
            let revision = state
                .repository_engine
                .materialize_revision(&state, &incarnation, head, spans)
                .await
                .unwrap();
            let _other_permit = state.runtime_budgets.try_git_materialization().unwrap();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let started_tx = Mutex::new(Some(started_tx));
            let (release_tx, release_rx) = mpsc::channel();
            let release_rx = Mutex::new(release_rx);
            let count = AtomicUsize::new(0);
            let repo_path = Arc::new(Mutex::new(PathBuf::new()));
            let path_for_hook = repo_path.clone();
            let owner = operation::with_hook(
                &state,
                Box::new(move || {
                    if count.fetch_add(1, Ordering::SeqCst) + 1 != phase {
                        return;
                    }
                    started_tx.lock().unwrap().take().unwrap().send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                    let path = path_for_hook.lock().unwrap();
                    assert!(path.exists(), "repository removed beneath blocking child");
                    match outcome {
                        "panic" => panic!("injected blocking child panic"),
                        "failure" => {
                            if phase == 1 {
                                fs::write(path.join("objects"), "not a directory").unwrap();
                            } else {
                                fs::remove_file(path.join("refs/heads/main")).unwrap();
                            }
                        }
                        _ => {}
                    }
                }),
            );
            // Observe the path without keeping the operation resources alive.
            let path = operation::repository(&owner);
            *repo_path.lock().unwrap() = path.clone();
            let operation_state = state.clone();
            let (completed_tx, completed_rx) = tokio::sync::oneshot::channel();
            let request = tokio::spawn(async move {
                operation::supervise(async move {
                    let result = materialize_owned_git_head_bundle(
                        &operation_state,
                        revision,
                        4 * 1024 * 1024,
                        owner,
                    )
                    .await;
                    completed_tx.send(result.is_ok()).unwrap();
                    result
                })
                .await
            });
            tokio::time::timeout(Duration::from_secs(10), started_rx)
                .await
                .unwrap()
                .unwrap();
            request.abort();
            assert!(request.await.unwrap_err().is_cancelled());
            assert!(path.exists());
            assert!(state.runtime_budgets.try_git_materialization().is_err());
            release_tx.send(()).unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(10), completed_rx)
                    .await
                    .unwrap()
                    .unwrap(),
                outcome == "success",
            );
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if let Ok(permit) = state.runtime_budgets.try_git_materialization() {
                        assert!(
                            !path.exists(),
                            "capacity returned before repository cleanup"
                        );
                        drop(permit);
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
    }
}

#[test]
fn manual_bundle_inspection_reads_the_requested_workflow_at_the_pinned_commit() {
    let state = AppState::test_state();
    let source = tempfile::tempdir().unwrap();
    run_git(
        None,
        &["init", "-b", "main", source.path().to_str().unwrap()],
        "initialize manual run source",
    )
    .unwrap();
    run_git(
        Some(source.path()),
        &["config", "user.email", "scope@test.invalid"],
        "configure source repository email",
    )
    .unwrap();
    run_git(
        Some(source.path()),
        &["config", "user.name", "Scope test"],
        "configure source repository name",
    )
    .unwrap();
    fs::create_dir_all(source.path().join(".scope/runs")).unwrap();
    fs::write(
        source.path().join(".scope/runs/checks.yml"),
        format!(
            "name: Checks\non:\n  manual: true\ncaches: []\ncontainer:\n  image: alpine@sha256:{}\ntimeout: 5m\njobs:\n  checks:\n    steps:\n      - name: Test\n        run: 'true'\n",
            "a".repeat(64)
        ),
    )
    .unwrap();
    run_git(
        Some(source.path()),
        &["add", "."],
        "stage manual run source",
    )
    .unwrap();
    run_git(
        Some(source.path()),
        &["commit", "-m", "manual run source"],
        "commit manual run source",
    )
    .unwrap();
    let git_oid = String::from_utf8(
        run_git_output(
            Some(source.path()),
            &["rev-parse", "HEAD"],
            "read manual run commit",
        )
        .unwrap()
        .stdout,
    )
    .unwrap();
    let git_oid = git_oid.trim();
    let bundle_path = source.path().join("source.bundle");
    run_git(
        Some(source.path()),
        &["bundle", "create", bundle_path.to_str().unwrap(), "HEAD"],
        "create manual run bundle",
    )
    .unwrap();

    let parsed = inspect_manual_run_bundle(
        &state.data_dir.join("manual-run-inspection-test"),
        &fs::read(bundle_path).unwrap(),
        git_oid,
        "checks",
    )
    .unwrap();
    let revision = parsed.into_revision("owner/repo").unwrap();

    assert!(revision.definition().triggers().manual());
    assert_eq!(
        revision.workflow().path().as_str(),
        "/.scope/runs/checks.yml"
    );
}

#[test]
fn workflow_blob_inspection_distinguishes_absence_invalid_type_oversize_and_read_failure() {
    let source = tempfile::tempdir().unwrap();
    run_git(
        Some(source.path()),
        &["init", "-q"],
        "initialize workflow fixture",
    )
    .unwrap();
    fs::create_dir_all(source.path().join(".scope/runs/directory.yml")).unwrap();
    fs::write(
        source.path().join(".scope/runs/directory.yml/nested"),
        "nested",
    )
    .unwrap();
    fs::write(source.path().join(".scope/runs/good.yml"), "name: Checks\n").unwrap();
    fs::write(
        source.path().join(".scope/runs/large.yml"),
        vec![b'x'; scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES + 1],
    )
    .unwrap();
    run_git(Some(source.path()), &["add", "."], "stage workflow fixture").unwrap();
    run_git(
        Some(source.path()),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "-qm",
            "workflows",
        ],
        "commit workflow fixture",
    )
    .unwrap();
    let bare = source.path().join(".git");
    assert_eq!(
        git_blob(&bare, "HEAD", ".scope/runs/missing.yml").unwrap(),
        None
    );
    assert_eq!(
        git_blob(&bare, "HEAD", ".scope/runs/good.yml")
            .unwrap()
            .unwrap(),
        b"name: Checks\n"
    );
    assert_eq!(
        git_blob(&bare, "HEAD", ".scope/runs/directory.yml")
            .unwrap_err()
            .status(),
        axum::http::StatusCode::BAD_REQUEST
    );
    assert!(
        git_blob(&bare, "HEAD", ".scope/runs/large.yml")
            .unwrap_err()
            .public_message()
            .contains("exceeds")
    );
    assert_eq!(
        git_blob(&bare, "invalid-object", ".scope/runs/good.yml")
            .unwrap_err()
            .status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    let oid = String::from_utf8(
        run_git_output(
            Some(source.path()),
            &["rev-parse", "HEAD:.scope/runs/good.yml"],
            "find workflow blob",
        )
        .unwrap()
        .stdout,
    )
    .unwrap();
    let oid = oid.trim();
    fs::remove_file(bare.join("objects").join(&oid[..2]).join(&oid[2..])).unwrap();
    assert_eq!(
        git_blob(&bare, "HEAD", ".scope/runs/good.yml")
            .unwrap_err()
            .status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
}

async fn bundle_bytes(source: MaterializedRunSource) -> Vec<u8> {
    axum::body::to_bytes(source.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}
