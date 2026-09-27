use super::*;
use crate::git::import::git_snapshot_from_ref;

#[tokio::test]
async fn private_request_run_source_supplies_its_old_main_base_before_runner_checkout() {
    let state = AppState::test_state();
    let (main_source, repository) = git_head_fixture(&state).await;
    let (_, pinned_main, _) = main_source.logical_git_head().unwrap();
    let base_oid = pinned_main.head_oid.clone();
    let first_span = main_source.logical_git_head().unwrap().2[0].clone();
    let incarnation = state
        .metadata
        .repositories()
        .git_push_context("owner", "repo", "user-owner")
        .await
        .unwrap()
        .unwrap()
        .incarnation;

    run_git(
        Some(repository.path()),
        &["checkout", "-b", "request"],
        "start request branch",
    )
    .unwrap();
    fs::write(repository.path().join("README.md"), "request content").unwrap();
    run_git(
        Some(repository.path()),
        &["commit", "-am", "request change"],
        "commit request change",
    )
    .unwrap();
    let (snapshot, thin_bytes) =
        git_snapshot_from_ref(repository.path(), "refs/heads/request", Some(&base_oid)).unwrap();
    let source_key = scope_storage::object_key(&snapshot);
    state
        .object_store
        .put(&source_key, thin_bytes.clone())
        .await
        .unwrap();
    let thin_path = repository.path().join("thin.bundle");
    fs::write(&thin_path, &thin_bytes).unwrap();
    let thin_clone = repository.path().join("thin.git");
    let failed = std::process::Command::new("git")
        .args(["clone", "--bare", "--no-local"])
        .arg(&thin_path)
        .arg(&thin_clone)
        .output()
        .unwrap();
    assert!(
        !failed.status.success(),
        "the fixture must be a thin bundle"
    );

    run_git(
        Some(repository.path()),
        &["checkout", "main"],
        "return to main",
    )
    .unwrap();
    fs::write(repository.path().join("README.md"), "new main content").unwrap();
    run_git(
        Some(repository.path()),
        &["commit", "-am", "advance main"],
        "advance main after request branch",
    )
    .unwrap();
    let advanced = git_push_from_repo(&state, "owner/repo", repository.path(), Some(pinned_main))
        .await
        .unwrap();
    let advanced_head = GitHead {
        change_version: 2,
        ..advanced.stored.head
    };
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests("owner/repo", move |repo| {
            repo.record.change_version = 2;
            repo.git_head = Some(advanced_head);
            repo.git_pack_spans = vec![first_span, advanced.stored.pack_span];
        })
        .await
        .unwrap();

    let materialized =
        materialize_request_git_bundle(&state, &incarnation, &snapshot, &base_oid, 4 * 1024 * 1024)
            .await
            .unwrap();
    let bundle = repository.path().join("standalone.bundle");
    fs::write(&bundle, bundle_bytes(materialized).await).unwrap();
    let checkout = repository.path().join("runner.git");
    run_git(
        None,
        &[
            "clone",
            "--bare",
            "--no-local",
            bundle.to_str().unwrap(),
            checkout.to_str().unwrap(),
        ],
        "clone request run source into an empty runner",
    )
    .unwrap();
    let cloned_head = String::from_utf8(
        run_git_output(
            Some(&checkout),
            &["rev-parse", "refs/heads/main"],
            "read restored request head",
        )
        .unwrap()
        .stdout,
    )
    .unwrap();
    assert_eq!(cloned_head.trim(), snapshot.git_oid);
    assert_eq!(
        run_git_output(
            Some(&checkout),
            &["show", "main:README.md"],
            "read restored request content",
        )
        .unwrap()
        .stdout,
        b"request content"
    );
}

#[tokio::test]
async fn private_request_full_projection_snapshot_survives_later_git_main() {
    for main_pushed_after_snapshot in [false, true] {
        let state = AppState::test_state();
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
        let incarnation = state
            .metadata
            .repositories()
            .git_push_context("owner", "repo", "user-owner")
            .await
            .unwrap()
            .unwrap()
            .incarnation;
        assert!(
            state
                .metadata
                .repositories()
                .repository_content_source(&incarnation)
                .await
                .unwrap()
                .0
                .is_none()
        );

        let repository = tempfile::tempdir().unwrap();
        run_git(
            None,
            &["init", "-b", "main", repository.path().to_str().unwrap()],
            "initialize projected request source",
        )
        .unwrap();
        run_git(
            Some(repository.path()),
            &["config", "user.email", "scope@test.invalid"],
            "configure projected source email",
        )
        .unwrap();
        run_git(
            Some(repository.path()),
            &["config", "user.name", "Scope test"],
            "configure projected source name",
        )
        .unwrap();
        fs::write(repository.path().join("README.md"), "projected base").unwrap();
        run_git(
            Some(repository.path()),
            &["add", "."],
            "stage projected base",
        )
        .unwrap();
        run_git(
            Some(repository.path()),
            &["commit", "-m", "projected base"],
            "commit projected base",
        )
        .unwrap();
        let base_oid = String::from_utf8(
            run_git_output(
                Some(repository.path()),
                &["rev-parse", "HEAD"],
                "read projected base",
            )
            .unwrap()
            .stdout,
        )
        .unwrap();
        let base_oid = base_oid.trim().to_string();
        run_git(
            Some(repository.path()),
            &["checkout", "-b", "request"],
            "start projected request",
        )
        .unwrap();
        fs::write(repository.path().join("README.md"), "projected request").unwrap();
        run_git(
            Some(repository.path()),
            &["commit", "-am", "projected request"],
            "commit projected request",
        )
        .unwrap();
        let (snapshot, bytes) =
            git_snapshot_from_ref(repository.path(), "refs/heads/request", None).unwrap();
        let source = RunSource::request_git_snapshot(snapshot.clone(), base_oid.clone()).unwrap();
        assert_eq!(source.request_git_source().unwrap().1, base_oid);
        state
            .object_store
            .put(&scope_storage::object_key(&snapshot), bytes)
            .await
            .unwrap();
        if main_pushed_after_snapshot {
            let accepted_main = tempfile::tempdir().unwrap();
            run_git(
                None,
                &["init", "-b", "main", accepted_main.path().to_str().unwrap()],
                "initialize accepted Git main",
            )
            .unwrap();
            run_git(
                Some(accepted_main.path()),
                &["config", "user.email", "scope@test.invalid"],
                "configure accepted main email",
            )
            .unwrap();
            run_git(
                Some(accepted_main.path()),
                &["config", "user.name", "Scope test"],
                "configure accepted main name",
            )
            .unwrap();
            fs::write(accepted_main.path().join("README.md"), "unrelated Git main").unwrap();
            run_git(Some(accepted_main.path()), &["add", "."], "stage Git main").unwrap();
            run_git(
                Some(accepted_main.path()),
                &["commit", "-m", "accept unrelated Git main"],
                "commit Git main",
            )
            .unwrap();
            let accepted = git_push_from_repo(&state, "owner/repo", accepted_main.path(), None)
                .await
                .unwrap();
            let head = GitHead {
                change_version: 1,
                ..accepted.stored.head
            };
            state
                .metadata
                .repositories()
                .mutate_repository_for_tests("owner/repo", move |repo| {
                    repo.record.change_version = 1;
                    repo.git_head = Some(head);
                    // The complete request snapshot must not read unrelated
                    // accepted Git history, even when that history cannot load.
                    repo.git_pack_spans = Vec::new();
                })
                .await
                .unwrap();
        }

        let result = materialize_request_git_bundle(
            &state,
            &incarnation,
            &snapshot,
            &base_oid,
            4 * 1024 * 1024,
        )
        .await
        .unwrap();
        let bundle = repository.path().join("projected.bundle");
        fs::write(&bundle, bundle_bytes(result).await).unwrap();
        let checkout = repository.path().join("runner.git");
        run_git(
            None,
            &[
                "clone",
                "--bare",
                "--no-local",
                bundle.to_str().unwrap(),
                checkout.to_str().unwrap(),
            ],
            "clone projected request source",
        )
        .unwrap();
        let restored_head = String::from_utf8(
            run_git_output(
                Some(&checkout),
                &["rev-parse", "refs/heads/main"],
                "read projected request head",
            )
            .unwrap()
            .stdout,
        )
        .unwrap();
        assert_eq!(restored_head.trim(), snapshot.git_oid);
        let restored_base = String::from_utf8(
            run_git_output(
                Some(&checkout),
                &["rev-parse", "main^"],
                "read projected request base",
            )
            .unwrap()
            .stdout,
        )
        .unwrap();
        assert_eq!(restored_base.trim(), base_oid);
    }
}
