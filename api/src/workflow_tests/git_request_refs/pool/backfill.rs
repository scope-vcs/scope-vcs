use super::*;
use crate::{
    git::pool_install::install_chain, use_cases::pool_bundle_backfill::backfill_one_bundle,
};
use scope_domain::requests::{RecordRequestRevisionInput, RequestRevisionGitFacts};

pub(super) fn empty_repo() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    run_git(
        None,
        &["init", "--bare", directory.path().to_str().unwrap()],
        "initialize empty pool replica",
    )
    .unwrap();
    directory
}

pub(super) async fn save_bundle_revision(
    state: &AppState,
    request_id: &str,
    source: &FsPath,
    base: Option<&str>,
) -> String {
    let request = stored_request(state, request_id).await;
    let head = git_head_oid(source);
    let request_ref = scope_domain::requests::canonical_request_ref(&request.name);
    run_git(
        Some(source),
        &["update-ref", &request_ref, &head],
        "name stored bundle request ref",
    )
    .unwrap();
    let (snapshot, bytes) = git_snapshot_from_ref(source, &request_ref, base).unwrap();
    state
        .object_store
        .put(&scope_storage::object_key(&snapshot), bytes)
        .await
        .unwrap();
    state
        .metadata
        .requests()
        .record_request_revision(RecordRequestRevisionInput {
            request_id: request.id,
            actor_user_id: test_owner_id(),
            actor_can_edit: true,
            expected_old_head_oid: Some(request.head_oid.clone()),
            new_head_oid: head.clone(),
            git_snapshot: snapshot,
            git_facts: RequestRevisionGitFacts {
                contains_old_head: git_is_ancestor(
                    source,
                    &request.head_oid,
                    &head,
                    "fixture ancestry",
                )
                .unwrap(),
                contained_main_oid: None,
                contained_main_descends_from_base: false,
            },
            event_id: format!("revision_{request_id}_{head}"),
            body: None,
            now_unix: request.updated_at_unix + 1,
            new_base: None,
            new_base_stack_depth: None,
        })
        .await
        .unwrap();
    head
}

pub(super) async fn stored_packs(
    state: &AppState,
    request_id: &str,
) -> Vec<scope_domain::repository::git::GitPoolPack> {
    let listed = state
        .metadata
        .repositories()
        .git_pool_packs_for_requests(TEST_REPO_ID, &[request_id.into()])
        .await
        .unwrap();
    let ids = listed
        .iter()
        .map(|pack| pack.id.clone())
        .collect::<Vec<_>>();
    let mut storage = state
        .metadata
        .repositories()
        .git_pool_pack_storage(TEST_REPO_ID, &ids)
        .await
        .unwrap()
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    listed
        .into_iter()
        .map(|pack| scope_domain::repository::git::GitPoolPack {
            storage: storage.remove(&pack.id).unwrap(),
            id: pack.id,
            owner: scope_domain::repository::git::GitPoolPackOwner::Request {
                request_id: pack.request_id,
                scope: pack.scope,
            },
            head_oid: pack.head_oid,
            prerequisites: pack.prerequisites,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_bundle_backfill_preserves_rewritten_history_and_thin_main_prerequisites() {
    let (state, source, main) =
        super::super::super::push_intent_completion::published_git_fixture("backfill-chain").await;
    let request_id = start_request(&state, "backfill").await;
    run_git(
        Some(&source),
        &["checkout", "-b", "topic"],
        "start bundle chain",
    )
    .unwrap();
    let mut seed = 42_u64;
    let large = (0..1_100_000)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect::<Vec<_>>();
    fs::write(source.join("large.bin"), large).unwrap();
    run_git(Some(&source), &["add", "large.bin"], "stage large bundle").unwrap();
    commit_file(&source, "first.txt", "first\n");
    let first = save_bundle_revision(&state, &request_id, &source, None).await;
    commit_file(&source, "second.txt", "second\n");
    let second = save_bundle_revision(&state, &request_id, &source, Some(&first)).await;
    run_git(
        Some(&source),
        &["reset", "--hard", &main],
        "replace old history",
    )
    .unwrap();
    commit_file(&source, "replacement.txt", "replacement\n");
    let replacement = save_bundle_revision(&state, &request_id, &source, Some(&main)).await;
    let replica = state.replica_for_tests();
    for now in [100, 130, 160] {
        assert!(backfill_one_bundle(&replica, &|| Ok(now)).await.unwrap());
    }
    let verified = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert!(verified.failures.is_empty(), "{:?}", verified.failures);
    let before_rerun = stored_packs(&state, &request_id).await;
    assert!(!backfill_one_bundle(&state, &|| Ok(190)).await.unwrap());
    assert_eq!(stored_packs(&state, &request_id).await, before_rerun);
    assert_eq!(before_rerun.len(), 3);
    assert!(before_rerun.iter().any(|pack| matches!(
        pack.storage,
        scope_domain::repository::git::GitPoolPackStorage::Segment(_)
    )));
    assert!(before_rerun.iter().any(|pack| matches!(
        pack.storage,
        scope_domain::repository::git::GitPoolPackStorage::Inline { .. }
    )));
    let empty = empty_repo();
    crate::git::pool_install::install_main_head(
        &replica,
        &test_repo_incarnation(),
        empty.path(),
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .git_head
            .as_ref()
            .unwrap(),
    )
    .await
    .unwrap();
    let installed = install_chain(
        &replica,
        &test_repo_incarnation(),
        empty.path(),
        before_rerun,
    )
    .await
    .unwrap();
    assert_eq!(installed, 3);
    for head in [first, second, replacement] {
        assert!(has_commit(empty.path(), &head));
    }
    let thin_request = start_request(&state, "thin-main").await;
    save_bundle_revision(&state, &thin_request, &source, Some(&main)).await;
    assert!(backfill_one_bundle(&replica, &|| Ok(220)).await.unwrap());
    let thin_packs = stored_packs(&state, &thin_request).await;
    assert_eq!(thin_packs[0].prerequisites, vec![main]);
    let empty_thin = empty_repo();
    let repository = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    crate::git::pool_install::install_main_head(
        &replica,
        &test_repo_incarnation(),
        empty_thin.path(),
        repository.git_head.as_ref().unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        install_chain(
            &replica,
            &test_repo_incarnation(),
            empty_thin.path(),
            thin_packs
        )
        .await
        .unwrap(),
        1
    );
    assert!(has_commit(empty_thin.path(), &git_head_oid(&source)));
    let report = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert_eq!(
        (
            report.total.remaining,
            report.total.done,
            report.total.failed
        ),
        (0, 4, 0)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_bundle_backfill_reports_bad_bundles_and_keeps_view_packs_off_shared_shelves() {
    let (state, source, _) =
        super::super::super::push_intent_completion::published_git_fixture("backfill-failures")
            .await;
    let full = start_request(&state, "bad-bundle").await;
    run_git(
        Some(&source),
        &["checkout", "-b", "topic"],
        "start bad bundle",
    )
    .unwrap();
    commit_file(&source, "bad.txt", "bad\n");
    save_bundle_revision(&state, &full, &source, None).await;
    let revision = state
        .metadata
        .requests()
        .latest_request_revision(&full)
        .await
        .unwrap()
        .unwrap();
    state
        .object_store
        .put(
            &scope_storage::object_key(&revision.git_snapshot),
            b"corrupt".to_vec(),
        )
        .await
        .unwrap();
    start_request_for_author(&state, test_owner_id(), RequestActorRole::Owner).await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let view = crate::git::repository_git::RepositoryGit::of_repository(&repo)
        .view_repo(&state, repo.repo_config.views(), &ViewId::public())
        .await
        .unwrap();
    let view_main = git_head_oid(&view);
    let public_source = clone_test_repo(&view, "backfill-public-source", false);
    run_git(
        Some(&public_source),
        &["checkout", "-b", "topic"],
        "start view bundle",
    )
    .unwrap();
    commit_file(&public_source, "public.txt", "public\n");
    let public_head = save_bundle_revision(&state, REQUEST_ID, &public_source, None).await;
    for now in [100, 130] {
        backfill_one_bundle(&state, &|| Ok(now)).await.unwrap();
    }
    let report = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert_eq!(
        (
            report.total.remaining,
            report.total.done,
            report.total.failed
        ),
        (1, 1, 1)
    );
    assert!(!report.failures[0].reason.is_empty());
    let packs = stored_packs(&state, REQUEST_ID).await;
    assert!(packs[0].prerequisites.contains(&view_main));
    assert!(
        matches!(&packs[0].owner, scope_domain::repository::git::GitPoolPackOwner::Request {
        scope: scope_domain::repository::git::RequestPackScope::View(view), ..
    } if view == &ViewId::public())
    );
    let replica = state.replica_for_tests();
    let empty = empty_repo();
    run_git(
        Some(empty.path()),
        &[
            "fetch",
            "--no-tags",
            view.to_str().unwrap(),
            "+HEAD:refs/heads/main",
        ],
        "install view main before thin pack",
    )
    .unwrap();
    assert_eq!(
        install_chain(&replica, &test_repo_incarnation(), empty.path(), packs)
            .await
            .unwrap(),
        1
    );
    assert!(has_commit(empty.path(), &public_head));
    assert!(!has_commit(
        &object_repos(&state, &test_owner_id()).await.shared,
        &public_head
    ));
    state
        .object_store
        .delete(&scope_storage::object_key(&revision.git_snapshot))
        .await
        .unwrap();
    assert!(backfill_one_bundle(&state, &|| Ok(86530)).await.unwrap());
    let missing = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert_eq!(missing.total.failed, 1);
    assert_ne!(missing.failures[0].reason, report.failures[0].reason);
}
