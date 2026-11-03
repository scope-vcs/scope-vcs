use super::*;
use crate::git::request_refs::view_object_repos;
use scope_domain::repository::git::RequestPackScope;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restricted_revisions_use_isolated_view_shelves_across_replicas_and_generations() {
    let (state, _main) = test_state_with_mergeable_request("view-pool").await;
    let replica = state.replica_for_tests();
    let (source, remote, _server, first) = request_checkout(&state, "view-pool-source").await;
    let old_head = stored_request(&state, REQUEST_ID).await.main_ancestor_oid;
    let first_shelf = view_shelf(&state).await;
    assert!(has_commit(&first_shelf, &first));
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "request.txt",
        "second revision\n",
        "second",
    )
    .unwrap();
    let second = git_head_oid(&source);
    let packs = state
        .metadata
        .repositories()
        .git_pool_packs_for_requests(TEST_REPO_ID, &[REQUEST_ID.to_string()])
        .await
        .unwrap();
    assert_eq!(packs.len(), 2);
    for pack in &packs {
        assert_eq!(pack.scope, RequestPackScope::View(ViewId::public()));
        assert!(pack.prerequisites.contains(&old_head));
    }
    assert!(
        packs
            .iter()
            .any(|pack| pack.head_oid == second && pack.prerequisites.contains(&first))
    );
    let canonical = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .git_head
        .unwrap()
        .head_oid;
    assert_ne!(canonical, old_head);
    assert!(
        packs
            .iter()
            .all(|pack| !pack.prerequisites.contains(&canonical))
    );
    let full = object_repos(&state, &public_user_id()).await;
    assert!(!has_commit(&full.shared, &second));
    assert!(!has_commit(&full.authored, &second));
    let staging = assert_restored_request_head(&replica, &second).await;
    let replica_shelf = view_shelf(&replica).await;
    assert!(has_commit(&replica_shelf, &first));
    assert!(has_commit(&replica_shelf, &second));
    assert!(
        !run_git_output(
            Some(&replica_shelf),
            &["show-ref", "--verify", REQUEST_REF],
            "check snapshot restore"
        )
        .unwrap()
        .status
        .success()
    );
    assert!(!staging.join("objects/info/alternates").exists());
    assert!(!has_commit(&staging, &canonical));
    fs::remove_dir_all(staging).unwrap();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let mut next = repo.repo_config.clone();
    let mut definitions = next.views.iter().cloned().collect::<Vec<_>>();
    definitions
        .iter_mut()
        .find(|definition| definition.id == ViewId::public())
        .unwrap()
        .name = "Outside".to_string();
    next.views = scope_domain::views::Views::new(definitions).unwrap();
    let body = push_intent_request_json_with_base(
        &repo.git_head.as_ref().unwrap().head_oid,
        repo_config_fingerprint(&repo.repo_config).unwrap(),
        next,
    );
    let changed = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/push-intents"),
        Some(&bearer_header()),
        Some(&body),
    )
    .await;
    assert_eq!(
        changed.status(),
        StatusCode::OK,
        "{}",
        response_json(changed).await
    );
    let next_shelf = view_shelf(&state).await;
    assert_ne!(first_shelf, next_shelf);
    assert!(!has_commit(&next_shelf, &second));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restricted_receive_pack_cannot_resolve_shared_shelf_objects_by_id() {
    let (state, _main) = test_state_with_mergeable_request("view-pool-graft").await;
    let (source, _remote, _server, old) = request_checkout(&state, "view-pool-graft-source").await;
    let shared = object_repos(&state, &test_owner_id()).await.shared;
    let shelf = view_shelf(&state).await;
    for (label, objects) in [("shared", &shared), ("unreadable-view-draft", &shelf)] {
        let hidden = git_input(
            objects,
            &["hash-object", "-w", "--stdin"],
            format!("private {label} content\n").as_bytes(),
        );
        let mut tree = git_stdout_text(&source, &["ls-tree", "HEAD"], "read public tree").unwrap();
        tree.push_str(&format!("100644 blob {hidden}\tstolen.txt\n"));
        let tree = git_input(&source, &["mktree", "--missing"], tree.as_bytes());
        let commit = format!(
            "tree {tree}\nparent {old}\nauthor Test <test@example.invalid> 1700000000 +0000\ncommitter Test <test@example.invalid> 1700000000 +0000\n\nreference hidden blob\n"
        );
        let head = git_input(
            &source,
            &["hash-object", "-t", "commit", "-w", "--stdin"],
            commit.as_bytes(),
        );
        let command = format!("{old} {head} {REQUEST_REF}\0report-status\n");
        let mut payload = format!("{:04x}{command}0000", command.len() + 4).into_bytes();
        let mut pack = std::process::Command::new("git");
        pack.current_dir(&source).args(["pack-objects", "--stdout"]);
        payload.extend(
            crate::git::command::git_command_output_with_timeout(
                &mut pack,
                Some(format!("{head}\n{tree}\n").into_bytes()),
                std::time::Duration::from_secs(5),
            )
            .unwrap(),
        );
        let response = router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/git/public/{TEST_REPO_ID}/git-receive-pack"))
                    .header(CONTENT_TYPE, "application/x-git-receive-pack-request")
                    .header(
                        AUTHORIZATION,
                        bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
                    )
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), 1_048_576).await.unwrap();
        assert!(
            String::from_utf8_lossy(&bytes)
                .contains(&format!("ng {REQUEST_REF} missing necessary objects")),
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        assert_eq!(stored_request(&state, REQUEST_ID).await.head_oid, old);
    }
}

async fn view_shelf(state: &AppState) -> PathBuf {
    let repo = find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    view_object_repos(
        state,
        &repo.incarnation(),
        &repo.repo_config,
        &ViewId::public(),
    )
    .await
    .unwrap()
    .objects
}

fn git_input(repo: &FsPath, args: &[&str], input: &[u8]) -> String {
    let mut command = std::process::Command::new("git");
    command.current_dir(repo).args(args);
    String::from_utf8(
        crate::git::command::git_command_output_with_timeout(
            &mut command,
            Some(input.to_vec()),
            std::time::Duration::from_secs(5),
        )
        .unwrap(),
    )
    .unwrap()
    .trim()
    .to_string()
}
