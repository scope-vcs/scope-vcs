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
