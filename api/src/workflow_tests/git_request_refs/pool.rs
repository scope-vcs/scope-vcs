use super::*;
use crate::use_cases::git_receive::{
    RequestRefSeeding,
    request_ref::{RequestPusher, RequestStagingKind, prepare_request_staging_repo},
};

async fn prepare_owner_push(state: &AppState, request_ref: &str) -> PathBuf {
    prepare_request_staging_repo(
        state,
        &test_repo_incarnation(),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        RequestPusher {
            actor_user_id: &test_owner_id(),
            address_view: &ViewId::private(),
        },
        RequestStagingKind::RequestRefsOnly,
        RequestRefSeeding::Pushed(&[request_ref.to_string()]),
    )
    .await
    .unwrap()
    .path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revision_pushed_to_one_replica_reaches_another_through_the_object_pool() {
    let (state, main_source, _) =
        super::super::push_intent_completion::published_git_fixture("request-pool").await;
    let replica = state.replica_for_tests();
    let bearer = bearer_header();
    let started = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer),
        Some(r#"{"name":"pooled","view":"private"}"#),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    let request_id = response_json(started).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let request_ref = "refs/heads/pooled";
    let _ = fs::remove_dir_all(prepare_owner_push(&replica, request_ref).await);

    let (first_origin, _first_server) = spawn_test_server(&state).await;
    let (second_origin, _second_server) = spawn_test_server(&replica).await;
    let first_remote = format!("{first_origin}/git/private/{TEST_REPO_ID}");
    let second_remote = format!("{second_origin}/git/private/{TEST_REPO_ID}");
    let source = clone_test_repo(&main_source, "request-pool-checkout", false);
    configure_bearer_header(&source, &first_remote, &bearer);
    configure_bearer_header(&source, &second_remote, &bearer);
    run_git(
        Some(&source),
        &["checkout", "-b", "pooled"],
        "create request branch",
    )
    .unwrap();
    push_change(
        &source,
        &first_remote,
        request_ref,
        "first.txt",
        "first replica\n",
        "first revision",
    )
    .unwrap();
    let first_head = git_head_oid(&source);
    push_change(
        &source,
        &second_remote,
        request_ref,
        "second.txt",
        "second replica\n",
        "second revision",
    )
    .unwrap();

    assert_eq!(
        stored_request(&state, &request_id).await.head_oid,
        git_head_oid(&source)
    );
    let packs = state
        .metadata
        .repositories()
        .git_pool_packs_after(TEST_REPO_ID, Some(0))
        .await
        .unwrap()
        .packs;
    assert_eq!(
        packs
            .iter()
            .map(|pack| pack.head_oid.as_str())
            .collect::<Vec<_>>(),
        [first_head.as_str(), git_head_oid(&source).as_str()]
    );
    let replica_pool =
        crate::git::storage::request_pool_repo_path(&replica, &test_repo_incarnation());
    assert!(
        run_git_output(
            Some(&replica_pool),
            &["cat-file", "-e", &format!("{first_head}^{{commit}}")],
            "read pooled revision",
        )
        .unwrap()
        .status
        .success()
    );
    let replica_store =
        crate::git::storage::request_ref_store_repo_path(&replica, &test_repo_incarnation());
    assert!(
        !run_git_output(
            Some(&replica_store),
            &["cat-file", "-e", &format!("{first_head}^{{commit}}")],
            "read restored revision",
        )
        .map(|output| output.status.success())
        .unwrap_or(false)
    );
}
