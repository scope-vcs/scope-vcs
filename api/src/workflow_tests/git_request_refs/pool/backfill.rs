use super::*;
use crate::use_cases::pool_bundle_backfill::{
    backfill, backfill_revision as verify, validate_backfill as finish,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_bundle_backfill_verifies_cold_readers_merges_stacks_corruption_and_reruns() {
    let (state, _, source) = super::super::super::custom_views::fixture("backfill").await;
    let requests = state.metadata.requests();
    let admin = state.metadata.admin();
    let sql = async |statement: String| admin.execute_for_tests(&statement).await.unwrap();
    let remove = async |id: &str| {
        let query = format!("DELETE FROM scope_git_pool_packs WHERE request_id = '{id}'");
        admin.execute_for_tests(&query).await.unwrap();
    };
    let push = |source, remote: &str, branch: &str, path: &str, content: &str| {
        push_change(source, remote, branch, path, content, path).unwrap()
    };
    let empty = backfill(&state, true).await.unwrap();
    assert!(finish(empty, false).is_err());
    finish(empty, true).unwrap();
    let parent = start_request(&state, "parent").await;
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer_header());
    let branch = "refs/heads/parent";
    let path = "revision.txt";
    let mut intermediate = String::new();
    for i in 0..33 {
        if i == 32 {
            commit_file(&source, "revision.txt", "intermediate\n");
            intermediate = git_head_oid(&source);
        }
        let content = format!("{i}\n");
        push(&source, &remote, branch, path, &content);
    }
    remove(&parent).await;
    assert_eq!(backfill(&state, false).await.unwrap(), [33, 33, 0]);
    let store = state.metadata.repositories();
    let revisions = store.bundle_batch(None, None).await.unwrap();
    let head = git_head_oid(&source);
    let mut revision = revisions.into_iter().find(|r| r.head_oid == head).unwrap();
    revision.old_head_oid = revision.head_oid.clone();
    revision.head_oid = intermediate;
    assert!(verify(&state, &revision, true).await.is_err());
    std::mem::swap(&mut revision.head_oid, &mut revision.old_head_oid);
    for _ in 0..2 {
        assert!(verify(&state, &revision, false).await.is_err());
    }
    let packs = store
        .git_pool_packs_for_requests(TEST_REPO_ID, std::slice::from_ref(&parent))
        .await
        .unwrap();
    assert_eq!(packs.len(), 33);
    let shadow = start_request(&state, "other-author").await;
    insert_member_user(&state).await;
    sql(format!("WITH draft AS (UPDATE scope_requests SET author_user_id = '{}', head_oid = (SELECT head_oid FROM scope_requests WHERE id = '{parent}') WHERE id = '{shadow}') UPDATE scope_git_pool_packs SET request_id = '{shadow}' WHERE request_id = '{parent}'", member_user_id())).await;
    assert_eq!(backfill(&state, true).await.unwrap(), [0, 0, 33]);
    sql(format!(
        "UPDATE scope_git_pool_packs SET request_id = '{parent}' WHERE request_id = '{shadow}'"
    ))
    .await;
    run_git(Some(&source), &["reset", "--hard", "HEAD~2"], "return to B").unwrap();
    run_git(
        Some(&source),
        &["push", "--force", &remote, "HEAD:refs/heads/parent"],
        "force push B",
    )
    .unwrap();
    assert_eq!(backfill(&state, true).await.unwrap(), [34, 0, 0]);
    let revision = requests.latest_request_revision(&parent).await.unwrap();
    let pin = revision.unwrap();
    submit_request(router(state.clone()), &parent, &bearer_header()).await;
    let child = super::super::super::requests::start_private_request(&state, "child", Some(serde_json::json!({
        "kind": "request", "request_id": parent, "revision_id": pin.id, "head_oid": pin.new_head_oid,
    }))).await;
    push(&source, &remote, "refs/heads/child", "child.txt", "child\n");
    sql(format!("UPDATE scope_request_revisions SET base_kind = 'Main', parent_request_id = NULL, parent_revision_id = NULL, parent_head_oid = NULL WHERE request_id = '{child}'")).await;
    assert_eq!(backfill(&state, true).await.unwrap(), [35, 0, 0]);
    sql(format!("UPDATE scope_request_revisions v SET base_kind = r.base_kind, parent_request_id = r.parent_request_id, parent_revision_id = r.parent_revision_id, parent_head_oid = r.parent_head_oid FROM scope_requests r WHERE r.id = v.request_id AND r.id = '{child}'")).await;

    let first = store.bundle_batch(None, Some(32)).await.unwrap();
    assert_eq!(first.len(), 32);
    sql(format!("UPDATE scope_requests SET base_kind = 'Main', parent_request_id = NULL, parent_revision_id = NULL, parent_head_oid = NULL, stack_status = 'main', stack_depth = 1 WHERE id = '{child}'; UPDATE scope_request_revisions SET position = position + 1000 WHERE request_id = '{parent}'")).await;

    let rest = store
        .bundle_batch(Some(&first.last().unwrap().source_id), Some(32))
        .await
        .unwrap();
    let ids: BTreeSet<_> = first
        .into_iter()
        .chain(rest)
        .map(|rev| rev.source_id)
        .collect();
    assert_eq!(ids.len(), 35);

    remove(&parent).await;
    assert_eq!(backfill(&state, false).await.unwrap(), [35, 33, 0]);
    sql(format!("UPDATE scope_git_pool_packs SET sealed_pack = decode('00', 'hex') WHERE request_id = '{child}'")).await;
    assert_eq!(backfill(&state, true).await.unwrap(), [34, 0, 1]);
    start_request_for_author(&state, test_owner_id(), RequestActorRole::Owner).await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let base = crate::git::repository_git::RepositoryGit::of_repository(&repo)
        .view_repo(&state, repo.repo_config.views(), &ViewId::public())
        .await
        .unwrap();
    let full_head = &repo.git_head.unwrap().head_oid;
    assert_ne!(&git_head_oid(&base), full_head);
    let public = clone_test_repo(&base, "backfill-public", false);
    let public_remote = remote.replace("/git/private/", "/git/public/");
    configure_bearer_header(&public, &public_remote, &bearer_header());
    for content in ["public\n", "next public\n"] {
        push(&public, &public_remote, REQUEST_REF, "README.md", content);
    }
    let public_head = git_head_oid(&public);
    remove(REQUEST_ID).await;
    assert_eq!(backfill(&state, false).await.unwrap(), [37, 3, 0]);
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
    assert_eq!(backfill(&state, true).await.unwrap(), [37, 0, 0]);
    let member = member_user_id();
    sql(format!("INSERT INTO scope_request_invitees (request_id, user_id, invited_by_user_id, created_at_unix) VALUES ('{REQUEST_ID}', '{member}', NULL, 3)")).await;
    let invited_replica = state.replica_for_tests();
    let staging = prepare_push(&invited_replica, &member, REQUEST_REF).await;
    assert!(has_commit(&staging, &public_head));
    submit_request(router(state.clone()), REQUEST_ID, &bearer_header()).await;
    let merged = api_request(
        router(state.replica_for_tests()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/merge"),
        Some(&bearer_header()),
        None,
    )
    .await;
    let merged = expect_json(merged, StatusCode::OK).await;
    assert_eq!(merged["request"]["state"], "Merged");
}
