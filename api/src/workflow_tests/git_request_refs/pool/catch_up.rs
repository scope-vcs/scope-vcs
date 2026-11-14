use super::*;
use crate::git::request_refs::{PoolInstall, PoolRepos, ensure_local_bare_repo, packs_to_catch_up};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reader_holding_part_of_a_request_walks_only_the_packs_it_lacks() {
    let (state, main_source, _) =
        super::super::super::push_intent_completion::published_git_fixture("request-pool-partial")
            .await;
    let request_id = start_request(&state, "partial").await;
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    let source = clone_test_repo(&main_source, "request-pool-partial-checkout", false);
    configure_bearer_header(&source, &remote, &bearer_header());
    run_git(
        Some(&source),
        &["checkout", "-b", "partial"],
        "create request branch",
    )
    .unwrap();
    let push = |revision: usize| {
        push_change(
            &source,
            &remote,
            "refs/heads/partial",
            "partial.txt",
            &format!("revision {revision}\n"),
            &format!("revision {revision}"),
        )
        .unwrap();
        git_head_oid(&source)
    };
    let mut heads = (0..3).map(push).collect::<Vec<_>>();

    let reader_replica = state.replica_for_tests();
    let request = stored_request(&reader_replica, &request_id).await;
    let revision = reader_replica
        .metadata
        .requests()
        .latest_request_revision(&request_id)
        .await
        .unwrap()
        .unwrap();
    crate::git::request_refs::with_request_revision_store_repo(
        &reader_replica,
        &test_repo_incarnation(),
        &request,
        &revision,
        |_, _| Ok(()),
    )
    .await
    .unwrap();
    let repo = find_repo(&reader_replica, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let incarnation = repo.incarnation();
    let repos = object_repos(&reader_replica, &test_owner_id()).await;
    let directory = tempfile::tempdir().unwrap();
    let reader = ensure_local_bare_repo(directory.path().join("reader.git")).unwrap();
    repos.add_alternates_to(&reader).unwrap();
    assert!(has_commit(&reader, &heads[2]));
    let owner = test_owner_id();
    let install = PoolInstall {
        incarnation: &incarnation,
        config: &repo.repo_config,
        repos: PoolRepos::Full(&repos, &owner, &reader),
    };

    heads.extend((3..5).map(push));
    let lineage = reader_replica
        .metadata
        .repositories()
        .git_pool_request_lineage(TEST_REPO_ID, std::slice::from_ref(&request_id))
        .await
        .unwrap()
        .into_iter()
        .map(|request| (request.id.clone(), request))
        .collect();
    let walked = packs_to_catch_up(
        &reader_replica,
        &install,
        &lineage,
        std::slice::from_ref(&heads[4]),
    )
    .await
    .unwrap()
    .into_iter()
    .map(|(_, pack)| pack.head_oid)
    .collect::<BTreeSet<_>>();
    assert_eq!(walked, heads[3..].iter().cloned().collect());
}
