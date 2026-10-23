use super::push_intent_completion::published_git_fixture;
use super::*;
use scope_domain::repository::git::{GitHead, GitPoolPackStorage};

pub(super) async fn main_pool_packs(
    state: &AppState,
) -> Vec<scope_postgres::db::ListedMainPoolPack> {
    state
        .metadata
        .repositories()
        .main_pool_packs(TEST_REPO_ID)
        .await
        .unwrap()
}

pub(super) async fn assert_pool_pack_discarded(
    state: &AppState,
    pack: &scope_domain::repository::git::GitPoolPack,
) {
    assert!(
        main_pool_packs(state)
            .await
            .iter()
            .all(|listed| listed.id != pack.id)
    );
    if let Some(segment) = pack.storage.segment() {
        assert!(
            state
                .git_segment_store
                .restore_to(TEST_REPO_ID, segment, tokio::io::sink())
                .await
                .is_err()
        );
    }
}

pub(super) async fn forget_main_pool_packs(state: &AppState) {
    state
        .metadata
        .admin()
        .execute_for_tests("DELETE FROM scope_git_pool_packs WHERE request_id IS NULL")
        .await
        .unwrap();
}

async fn push_main_update(state: &AppState, source: &FsPath, label: &str, content: &str) -> String {
    fs::write(source.join("README.md"), content).unwrap();
    run_git(Some(source), &["add", "README.md"], "stage main update").unwrap();
    commit_all(source, label);
    let bare = clone_test_repo(source, &format!("{label}-bare"), true);
    let update = reviewed_update_from_staging_repo(
        state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &bare,
        &test_owner_id(),
        repo_config(ViewId::public()),
        ReviewedUpdateMode::ReadyPush,
    )
    .await
    .unwrap();
    persist_test_update(state, update).await.unwrap();
    git_head_oid(source)
}

async fn accepted_head(state: &AppState) -> GitHead {
    find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .git_head
        .unwrap()
}

#[tokio::test]
async fn a_main_push_registers_an_inline_main_pool_pack_on_the_previous_head() {
    let (state, source, first_head) = published_git_fixture("main-pool-inline").await;
    let second_head =
        push_main_update(&state, &source, "main-pool-inline-update", "second\n").await;

    let listed = main_pool_packs(&state).await;
    assert_eq!(
        listed
            .iter()
            .map(|pack| (
                pack.first_sequence,
                pack.last_sequence,
                pack.head_oid.clone(),
                pack.prerequisites.clone()
            ))
            .collect::<Vec<_>>(),
        [
            (1, 1, first_head.clone(), Vec::new()),
            (2, 2, second_head, vec![first_head]),
        ]
    );
    let storage = state
        .metadata
        .repositories()
        .git_pool_pack_storage(
            TEST_REPO_ID,
            &listed
                .iter()
                .map(|pack| pack.id.clone())
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap();
    assert_eq!(storage.len(), 2);
    assert!(
        storage
            .iter()
            .all(|(_, storage)| matches!(storage, GitPoolPackStorage::Inline { .. }))
    );
}

#[tokio::test]
async fn a_fresh_replica_restores_and_catches_up_main_from_the_pool() {
    let (state, source, first_head) = published_git_fixture("main-pool-replica").await;
    let replica = state.replica_for_tests();
    let incarnation = test_repo_incarnation();
    assert!(
        !replica
            .repository_engine
            .repository_path(&incarnation)
            .exists()
    );

    let restored = replica
        .repository_engine
        .materialize_repository(&replica, &incarnation, &accepted_head(&state).await)
        .await
        .unwrap();
    assert_eq!(
        git_stdout_text(
            &restored,
            &["rev-parse", DEFAULT_GIT_BRANCH],
            "read restored main"
        )
        .unwrap()
        .trim(),
        first_head
    );
    drop(restored);

    let second_head =
        push_main_update(&state, &source, "main-pool-replica-update", "second\n").await;
    let caught_up = replica
        .repository_engine
        .materialize_repository(&replica, &incarnation, &accepted_head(&state).await)
        .await
        .unwrap();
    assert_eq!(
        git_stdout_text(
            &caught_up,
            &["rev-parse", DEFAULT_GIT_BRANCH],
            "read caught-up main"
        )
        .unwrap()
        .trim(),
        second_head
    );
    assert_eq!(
        git_stdout_text(
            &caught_up,
            &["show", &format!("{DEFAULT_GIT_BRANCH}:README.md")],
            "read caught-up content"
        )
        .unwrap(),
        "second\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_prepared_main_push_does_not_block_another_push_and_then_fails_as_stale() {
    let (state, source, _first_head) = published_git_fixture("main-pool-held-push").await;
    let base = accepted_head(&state).await;
    let held_checkout = clone_test_repo(&source, "main-pool-held-push-checkout", false);
    commit_file(&held_checkout, "held.txt", "held\n");
    let held_staging = clone_test_repo(&held_checkout, "main-pool-held-push-staging", true);
    let mut held = reviewed_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &held_staging,
        &test_owner_id(),
        repo_config(ViewId::public()),
        ReviewedUpdateMode::ReadyPush,
    )
    .await
    .unwrap();
    held.base_git_frontier = Some(Some(base.frontier()));
    let held_pack = held.staged_pack.pack.clone();
    let crate::git::pool_pack::LocalPoolPack::Files(held_files) = &held.staged_pack.local else {
        panic!("a small main push stages an inline pool pack");
    };
    let held_local_pack = held_files.pack.clone();
    assert!(held_local_pack.exists());

    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer_header());
    commit_file(&source, "other.txt", "other\n");
    configure_push_intent_header(&state, &source, &remote, &test_owner_id()).await;
    let pushing = source.to_path_buf();
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tokio::task::spawn_blocking(move || {
            run_git(
                Some(&pushing),
                &["push", &remote, &format!("HEAD:{DEFAULT_GIT_BRANCH}")],
                "push a disjoint main update while another push is prepared",
            )
        }),
    )
    .await
    .expect("a prepared main push must not block another push")
    .unwrap()
    .unwrap();
    assert_eq!(
        live_file_content(&state, "/other.txt").await.as_deref(),
        Some("other\n")
    );

    let error = persist_test_update(&state, held).await.unwrap_err();
    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(
        error.into_public_message(),
        crate::git::import::STALE_MAIN_PUSH_ERROR
    );
    assert_pool_pack_discarded(&state, &held_pack).await;
    assert!(!held_local_pack.exists());
    assert_eq!(live_file_content(&state, "/held.txt").await, None);
    assert_eq!(main_pool_packs(&state).await.len(), 2);
}
