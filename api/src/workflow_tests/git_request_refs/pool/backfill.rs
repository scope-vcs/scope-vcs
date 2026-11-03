use super::*;
use crate::use_cases::pool_bundle_backfill::{backfill, backfill_revision};
async fn remove_packs(state: &AppState, id: &str) {
    let sql = format!("DELETE FROM scope_git_pool_packs WHERE request_id = '{id}'");
    let admin = state.metadata.admin();
    admin.execute_for_tests(&sql).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_bundle_backfill_verifies_cold_readers_merges_stacks_corruption_and_reruns() {
    let (state, _, source) = super::super::super::custom_views::fixture("backfill").await;
    let requests = state.metadata.requests();
    let parent = start_request(&state, "parent").await;
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer_header());
    let mut intermediate = String::new();
    for i in 0..17 {
        if i == 16 {
            commit_file(&source, "revision.txt", "intermediate\n");
            intermediate = git_head_oid(&source);
        }
        push_change(
            &source,
            &remote,
            "refs/heads/parent",
            "revision.txt",
            &format!("{i}\n"),
            "revision",
        )
        .unwrap();
    }
    remove_packs(&state, &parent).await;
    assert_eq!(backfill(&state, false).await.unwrap(), [17, 17, 0]);
    let store = state.metadata.repositories();
    let mut missing_head = store.bundle_batch(0).await.unwrap().pop().unwrap();
    missing_head.old_head_oid = missing_head.head_oid.clone();
    missing_head.head_oid = intermediate;
    assert!(
        backfill_revision(&state, &missing_head, true)
            .await
            .is_err()
    );
    let shadow = start_request(&state, "other-author").await;
    insert_member_user(&state).await;
    state.metadata.admin().execute_for_tests(&format!("WITH draft AS (UPDATE scope_requests SET author_user_id = '{}', head_oid = (SELECT head_oid FROM scope_requests WHERE id = '{parent}') WHERE id = '{shadow}') UPDATE scope_git_pool_packs SET request_id = '{shadow}' WHERE request_id = '{parent}'", member_user_id())).await.unwrap();
    assert_eq!(backfill(&state, true).await.unwrap(), [0, 0, 17]);
    state
        .metadata
        .admin()
        .execute_for_tests(&format!(
            "UPDATE scope_git_pool_packs SET request_id = '{parent}' WHERE request_id = '{shadow}'"
        ))
        .await
        .unwrap();
    run_git(Some(&source), &["reset", "--hard", "HEAD~2"], "return to B").unwrap();
    run_git(
        Some(&source),
        &["push", "--force", &remote, "HEAD:refs/heads/parent"],
        "force push B",
    )
    .unwrap();
    assert_eq!(backfill(&state, true).await.unwrap(), [18, 0, 0]);
    let pin = requests
        .latest_request_revision(&parent)
        .await
        .unwrap()
        .unwrap();
    submit_request(router(state.clone()), &parent, &bearer_header()).await;
    let child = super::super::super::requests::start_private_request(&state, "child", Some(serde_json::json!({
        "kind": "request", "request_id": parent, "revision_id": pin.id, "head_oid": pin.new_head_oid,
    }))).await;
    push_change(
        &source,
        &remote,
        "refs/heads/child",
        "child.txt",
        "child\n",
        "child",
    )
    .unwrap();
    state.metadata.admin().execute_for_tests(&format!("UPDATE scope_requests SET base_kind = 'Main', parent_request_id = NULL, parent_revision_id = NULL, parent_head_oid = NULL, stack_status = 'main', stack_depth = 1 WHERE id = '{child}'")).await.unwrap();
    assert_eq!(store.bundle_batch(0).await.unwrap()[0].request_id, child);
    remove_packs(&state, &parent).await;
    assert_eq!(backfill(&state, false).await.unwrap(), [19, 17, 0]);
    state.metadata.admin().execute_for_tests(&format!("UPDATE scope_git_pool_packs SET sealed_pack = decode('00', 'hex') WHERE request_id = '{child}'")).await.unwrap();
    assert_eq!(backfill(&state, true).await.unwrap(), [18, 0, 1]);
    start_request_for_author(&state, test_owner_id(), RequestActorRole::Owner).await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let base = crate::git::repository_git::RepositoryGit::of_repository(&repo)
        .view_repo(&state, repo.repo_config.views(), &ViewId::public())
        .await
        .unwrap();
    assert_ne!(
        git_head_oid(&base),
        repo.git_head.as_ref().unwrap().head_oid
    );
    let public = clone_test_repo(&base, "backfill-public", false);
    let public_remote = remote.replace("/git/private/", "/git/public/");
    configure_bearer_header(&public, &public_remote, &bearer_header());
    for content in ["public\n", "next public\n"] {
        push_change(
            &public,
            &public_remote,
            REQUEST_REF,
            "README.md",
            content,
            "public",
        )
        .unwrap();
    }
    let public_head = git_head_oid(&public);
    remove_packs(&state, REQUEST_ID).await;
    assert_eq!(backfill(&state, false).await.unwrap(), [21, 3, 0]);
    assert!(!has_commit(
        &object_repos(&state, &test_owner_id()).await.shared,
        &public_head
    ));
    let snapshot = stored_request(&state, REQUEST_ID)
        .await
        .git_snapshot
        .unwrap();
    state
        .object_store
        .delete(&scope_storage::object_key(&snapshot))
        .await
        .unwrap();
    assert_eq!(backfill(&state, true).await.unwrap(), [21, 0, 0]);
    let member = member_user_id();
    state.metadata.admin().execute_for_tests(&format!("INSERT INTO scope_request_invitees (request_id, user_id, invited_by_user_id, created_at_unix) VALUES ('{REQUEST_ID}', '{member}', NULL, 3)")).await.unwrap();
    let invited_replica = state.replica_for_tests();
    let staging = prepare_push(&invited_replica, &member, REQUEST_REF).await;
    assert!(has_commit(&staging, &public_head));
    submit_request(router(state.clone()), REQUEST_ID, &bearer_header()).await;
    let replica = state.replica_for_tests();
    let merged = api_request(
        router(replica),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/merge"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(
        expect_json(merged, StatusCode::OK).await["request"]["state"],
        "Merged"
    );
}
