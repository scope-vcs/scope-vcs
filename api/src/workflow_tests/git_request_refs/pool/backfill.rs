use super::*;
use crate::use_cases::pool_bundle_backfill::backfill;
use scope_domain::requests::{RecordRequestRevisionInput, RequestRevisionGitFacts};

async fn save_revision(state: &AppState, id: &str, source: &FsPath) -> String {
    let request = stored_request(state, id).await;
    let head = git_head_oid(source);
    let refname = format!("refs/heads/{}", request.name);
    run_git(
        Some(source),
        &["update-ref", &refname, &head],
        "snapshot ref",
    )
    .unwrap();
    let (snapshot, bytes) = git_snapshot_from_ref(source, &refname, None).unwrap();
    state
        .object_store
        .put(&scope_storage::object_key(&snapshot), bytes)
        .await
        .unwrap();
    state
        .metadata
        .requests()
        .record_request_revision(RecordRequestRevisionInput {
            request_id: id.into(),
            actor_user_id: test_owner_id(),
            actor_can_edit: true,
            expected_old_head_oid: Some(request.head_oid),
            new_head_oid: head.clone(),
            git_snapshot: snapshot,
            git_facts: RequestRevisionGitFacts {
                contains_old_head: true,
                contained_main_oid: None,
                contained_main_descends_from_base: false,
            },
            event_id: format!("event_{id}_{}", request.updated_at_unix),
            body: None,
            now_unix: request.updated_at_unix + 1,
            new_base: None,
            new_base_stack_depth: None,
        })
        .await
        .unwrap();
    head
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_bundle_backfill_verifies_long_chains_stacks_corruption_privacy_and_reruns() {
    let (state, source, _) =
        super::super::super::push_intent_completion::published_git_fixture("backfill").await;
    let requests = state.metadata.requests();
    let parent = start_request(&state, "parent").await;
    for i in 0..17 {
        commit_file(&source, "revision.txt", &format!("{i}\n"));
        save_revision(&state, &parent, &source).await;
    }
    assert_eq!(backfill(&state, true).await.unwrap(), [0, 0, 17]);
    assert!(pooled_heads(&state, &parent).await.is_empty());
    assert_eq!(backfill(&state, false).await.unwrap(), [17, 17, 0]);
    let store = state.metadata.repositories();
    assert_eq!(backfill(&state, false).await.unwrap(), [17, 0, 0]);
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
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer_header());
    run_git(Some(&source), &["reset", "--hard", "HEAD~1"], "return to B").unwrap();
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
    assert_eq!(backfill(&state, false).await.unwrap(), [19, 0, 0]);
    state
        .metadata
        .admin()
        .execute_for_tests(&format!(
            "DELETE FROM scope_git_pool_packs WHERE request_id = '{parent}'"
        ))
        .await
        .unwrap();
    assert_eq!(backfill(&state, true).await.unwrap(), [0, 0, 19]);
    assert_eq!(backfill(&state, false).await.unwrap(), [19, 17, 0]);
    state.metadata.admin().execute_for_tests(&format!("UPDATE scope_git_pool_packs SET sealed_pack = decode('00', 'hex') WHERE request_id = '{child}'")).await.unwrap();
    assert_eq!(backfill(&state, true).await.unwrap(), [18, 0, 1]);
    let bad = stored_request(&state, &child).await.git_snapshot.unwrap();
    state
        .object_store
        .delete(&scope_storage::object_key(&bad))
        .await
        .unwrap();
    start_request_for_author(&state, test_owner_id(), RequestActorRole::Owner).await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let base = crate::git::repository_git::RepositoryGit::of_repository(&repo)
        .view_repo(&state, repo.repo_config.views(), &ViewId::public())
        .await
        .unwrap();
    let view_main = git_head_oid(&base);
    let public = clone_test_repo(&base, "backfill-public", false);
    commit_file(&public, "public.txt", "public\n");
    let first_public = save_revision(&state, REQUEST_ID, &public).await;
    commit_file(&public, "public.txt", "next public\n");
    let public_head = save_revision(&state, REQUEST_ID, &public).await;
    assert_eq!(backfill(&state, false).await.unwrap(), [20, 2, 1]);
    let packs = store
        .git_pool_packs_for_requests(TEST_REPO_ID, &[REQUEST_ID.to_string()])
        .await
        .unwrap();
    assert_eq!(packs.len(), 2);
    assert!(packs.iter().any(|p| p.head_oid == public_head
        && p.prerequisites.len() == 2
        && p.prerequisites.contains(&first_public)
        && p.prerequisites.contains(&view_main)));
    assert!(!has_commit(
        &object_repos(&state, &test_owner_id()).await.shared,
        &public_head
    ));
}
