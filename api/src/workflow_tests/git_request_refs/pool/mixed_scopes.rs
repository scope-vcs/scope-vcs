use super::*;
use scope_domain::repository::git::RequestPackScope;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_full_reader_installs_restricted_full_pack_with_its_view_prerequisite() {
    let (state, _main) = test_state_with_mergeable_request("mixed-scope-pool").await;
    let (source, remote, _server, first) = request_checkout(&state, "mixed-scope-source").await;
    submit_request(
        router(state.clone()),
        REQUEST_ID,
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    )
    .await;
    let full_remote = remote.replace("/git/public/", "/git/private/");
    configure_bearer_header(&source, &full_remote, &bearer_header());
    push_change(
        &source,
        &full_remote,
        REQUEST_REF,
        "request.txt",
        "full-view revision\n",
        "full-view revision",
    )
    .unwrap();
    let head = git_head_oid(&source);
    let packs = state
        .metadata
        .repositories()
        .git_pool_packs_for_requests(TEST_REPO_ID, &[REQUEST_ID.to_string()])
        .await
        .unwrap();
    assert!(packs.iter().any(
        |pack| pack.head_oid == first && pack.scope == RequestPackScope::View(ViewId::public())
    ));
    assert!(packs.iter().any(|pack| pack.head_oid == head
        && pack.scope == RequestPackScope::Full
        && pack.prerequisites.contains(&first)));
    let cold = state.replica_for_tests();
    let folder = fetch_folder(
        &cold,
        &authorization_headers(bearer_header()),
        &ViewId::private(),
    )
    .await;
    assert_eq!(ref_head(&folder, REQUEST_REF), head);
    run_git(
        Some(&folder),
        &["rev-list", "--objects", &head],
        "read mixed-scope pooled history",
    )
    .unwrap();
    assert!(has_commit(&folder, &first));
    for replica in [&state, &cold] {
        for author in [test_owner_id(), public_user_id(), member_user_id()] {
            let repos = object_repos(replica, &author).await;
            for shelf in [&repos.shared, &repos.authored] {
                assert!(!has_commit(shelf, &first));
                assert!(!has_commit(shelf, &head));
            }
        }
    }
}
