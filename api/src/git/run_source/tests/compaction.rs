use super::*;

#[tokio::test]
async fn run_pinned_to_an_earlier_main_head_materializes_after_main_moves_and_compacts() {
    let state = AppState::test_state();
    let (source, repository) = git_head_fixture(&state).await;
    let (_, pinned) = source.logical_git_head().unwrap();
    let incarnation = state
        .metadata
        .repositories()
        .git_push_state("owner", "repo", "user-owner")
        .await
        .unwrap()
        .unwrap()
        .incarnation;
    fs::write(repository.path().join("README.md"), "newer main").unwrap();
    run_git(
        Some(repository.path()),
        &["commit", "-am", "advance main"],
        "advance main past the pinned run",
    )
    .unwrap();
    let advanced = git_push_from_repo(&state, "owner/repo", repository.path(), Some(pinned))
        .await
        .unwrap();
    let advanced_head = GitHead {
        change_version: 2,
        ..advanced.head.clone()
    };
    record_accepted_main(&state, advanced_head.clone(), &advanced.pack.pack).await;
    let compacted = git_push_from_repo(&state, "owner/repo", repository.path(), None)
        .await
        .unwrap();
    state
        .metadata
        .admin()
        .execute_for_tests("DELETE FROM scope_git_pool_packs WHERE request_id IS NULL")
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .register_main_pool_pack_for_tests(
            "owner/repo",
            &GitPoolPack {
                owner: scope_domain::repository::git::GitPoolPackOwner::Main {
                    first_sequence: 1,
                    last_sequence: 2,
                },
                ..compacted.pack.pack.clone()
            },
        )
        .await
        .unwrap();
    state
        .repository_engine
        .delete_repository_cache(&incarnation)
        .unwrap();

    let materialized =
        materialize_accepted_git_head_bundle(&state, &incarnation, &source, 4 * 1024 * 1024)
            .await
            .unwrap();

    let bundle = repository.path().join("pinned.bundle");
    fs::write(&bundle, bundle_bytes(materialized).await).unwrap();
    let restored = repository.path().join("pinned.git");
    run_git(
        None,
        &[
            "clone",
            "--bare",
            "--no-local",
            bundle.to_str().unwrap(),
            restored.to_str().unwrap(),
        ],
        "clone the pinned run source",
    )
    .unwrap();
    assert_eq!(
        git_stdout_text(&restored, &["rev-parse", "main"], "read pinned head")
            .unwrap()
            .trim(),
        pinned.head_oid
    );
    assert_eq!(
        run_git_output(
            Some(&restored),
            &["show", "main:README.md"],
            "read pinned content"
        )
        .unwrap()
        .stdout,
        b"pinned run source"
    );
    let unknown =
        RunSource::accepted_git_head("owner/repo", GitHead::new("f".repeat(40), 1, 1)).unwrap();
    let error =
        materialize_accepted_git_head_bundle(&state, &incarnation, &unknown, 4 * 1024 * 1024)
            .await
            .err()
            .unwrap();
    assert!(
        error
            .operator_diagnostic()
            .contains("is not in the repository's accepted history"),
        "{}",
        error.operator_diagnostic()
    );
}
