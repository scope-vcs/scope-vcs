use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_fetch_advertises_and_installs_only_active_request_heads() {
    let (state, main, _) =
        super::super::super::push_intent_completion::published_git_fixture("fetch-open-pool").await;
    let source = clone_test_repo(&main, "fetch-open-source", false);
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer_header());
    let mut heads = Vec::new();
    for (name, terminal) in [
        ("active", None),
        ("closed", Some(false)),
        ("merged", Some(true)),
    ] {
        let id = start_request(&state, name).await;
        run_git(
            Some(&source),
            &["checkout", "-B", name, "origin/main"],
            "create independent request",
        )
        .unwrap();
        push_change(
            &source,
            &remote,
            &format!("refs/heads/{name}"),
            "request.txt",
            name,
            name,
        )
        .unwrap();
        let head = git_head_oid(&source);
        state
            .metadata
            .requests()
            .mutate_request_for_tests(&id, |request| {
                request.submitted_at_unix = Some(request.updated_at_unix);
                match terminal {
                    Some(false) => {
                        request.closed_at_unix = Some(request.updated_at_unix);
                        request.closed_by_user_id = Some(test_owner_id());
                    }
                    Some(true) => {
                        request.merged_at_unix = Some(request.updated_at_unix);
                        request.merged_by_user_id = Some(test_owner_id());
                        request.merged_head_oid = Some(head.clone());
                        request.merged_main_oid = Some(request.main_ancestor_oid.clone());
                    }
                    None => {}
                }
            })
            .await
            .unwrap();
        heads.push(head);
    }
    let replica = state.replica_for_tests();
    let folder = git_upload_pack_repo_for_request(
        &replica,
        &authorization_headers(bearer_header()),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &ViewId::private(),
    )
    .await
    .unwrap();
    let refs = git_stdout_text(
        &folder,
        &["for-each-ref", "--format=%(refname)"],
        "fetch advertised refs",
    )
    .unwrap();
    assert_eq!(
        refs.lines().collect::<Vec<_>>(),
        ["refs/heads/active", "refs/heads/main"]
    );
    let counts = git_stdout_text(
        &folder,
        &["count-objects", "-v"],
        "fetch folder object copies",
    )
    .unwrap();
    assert!(counts.lines().any(|line| line == "count: 0"));
    assert!(counts.lines().any(|line| line == "in-pack: 0"));
    assert!(has_commit(&folder, &heads[0]));
    for head in &heads[1..] {
        assert!(!has_commit(&folder, head));
    }
    let shelf = object_repos(&replica, &test_owner_id()).await.shared;
    assert!(has_commit(&shelf, &heads[0]));
    for head in &heads[1..] {
        assert!(!has_commit(&shelf, head));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rewritten_request_advertised_on_one_replica_is_fetchable_on_another() {
    let (state, _main) = test_state_with_mergeable_request("fetch-between-legs").await;
    let (source, remote, _server, old_head) =
        request_checkout(&state, "fetch-between-legs-source").await;
    let author = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let path = format!("/git/public/{TEST_REPO_ID}");
    let advertisement = api_request(
        router(state.clone()),
        "GET",
        &format!("{path}/info/refs?service=git-upload-pack"),
        Some(&author),
        None,
    )
    .await;
    assert_eq!(advertisement.status(), StatusCode::OK);
    let bytes = to_bytes(advertisement.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains(&format!("{old_head} {REQUEST_REF}")));
    run_git(
        Some(&source),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "--amend",
            "-m",
            "rewrite advertised head",
        ],
        "rewrite request",
    )
    .unwrap();
    let new_head = git_head_oid(&source);
    assert_ne!(old_head, new_head);
    run_git(
        Some(&source),
        &[
            "push",
            &format!("--force-with-lease={REQUEST_REF}:{old_head}"),
            &remote,
            &format!("HEAD:{REQUEST_REF}"),
        ],
        "replace advertised head",
    )
    .unwrap();
    let replica = state.replica_for_tests();
    let want = format!("want {old_head}");
    let payload = format!("{:04x}{want}00000009done\n", want.len() + 4);
    let response = router(replica.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("{path}/git-upload-pack"))
                .header(AUTHORIZATION, &author)
                .header(CONTENT_TYPE, "application/x-git-upload-pack-request")
                .body(Body::from(payload.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        bytes.windows(4).any(|bytes| bytes == b"PACK"),
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    insert_public_contributor(&state).await;
    let denied = router(replica)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("{path}/git-upload-pack"))
                .header(
                    AUTHORIZATION,
                    bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL),
                )
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    let denied = to_bytes(denied.into_body(), usize::MAX).await.unwrap();
    assert!(!denied.windows(4).any(|bytes| bytes == b"PACK"));
    assert!(String::from_utf8_lossy(&denied).contains("outside the visible refs"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unpushed_child_of_a_closed_parent_keeps_its_ref_without_the_parents() {
    let (state, main, _) =
        super::super::super::push_intent_completion::published_git_fixture("fetch-closed-parent")
            .await;
    let source = clone_test_repo(&main, "fetch-closed-parent-source", false);
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer_header());
    let parent_id = start_request(&state, "closed-parent").await;
    run_git(
        Some(&source),
        &["checkout", "-B", "closed-parent", "origin/main"],
        "create parent request",
    )
    .unwrap();
    push_change(
        &source,
        &remote,
        "refs/heads/closed-parent",
        "request.txt",
        "parent",
        "parent",
    )
    .unwrap();
    let head = git_head_oid(&source);
    let requests = state.metadata.requests();
    requests
        .mutate_request_for_tests(&parent_id, |request| {
            request.submitted_at_unix = Some(request.updated_at_unix);
        })
        .await
        .unwrap();
    let parent = requests
        .request_for_tests(&parent_id)
        .await
        .unwrap()
        .unwrap();
    let revision = requests
        .request_revision_with_head(&parent_id, &head)
        .await
        .unwrap()
        .unwrap();
    let child = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer_header()),
        Some(
            &serde_json::json!({
                "name": "open-child",
                "view": "private",
                "base": {
                    "kind": "request",
                    "request_id": parent.id,
                    "revision_id": revision.id,
                    "head_oid": head,
                },
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(child.status(), StatusCode::OK);
    let child = response_json(child).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let child = requests.request_for_tests(&child).await.unwrap().unwrap();
    let reader = scope_postgres::db::GitFetchReader {
        repo_id: &parent.repo_id,
        generation: "shared-head",
        view: &ViewId::private(),
        user_id: None,
    };
    let repositories = state.metadata.repositories();
    let now = unix_now();
    repositories
        .record_git_fetch_advertisement(&reader, "main", &[parent.clone()], now)
        .await
        .unwrap();
    repositories
        .record_git_fetch_advertisement(&reader, "main", &[child], now)
        .await
        .unwrap();
    repositories
        .record_git_fetch_advertisement(&reader, "main", &[parent.clone()], now)
        .await
        .unwrap();
    let advertised = repositories
        .advertised_git_fetch_heads(&reader, std::slice::from_ref(&head), now)
        .await
        .unwrap();
    assert_eq!(advertised[0].1.as_ref().map(Vec::len), Some(2));
    let closed = api_request(
        router(state.clone()),
        "DELETE",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{parent_id}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(closed.status(), StatusCode::OK);
    let replica = state.replica_for_tests();
    let folder = git_upload_pack_repo_for_request(
        &replica,
        &authorization_headers(bearer_header()),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &ViewId::private(),
    )
    .await
    .unwrap();
    let refs = git_stdout_text(
        &folder,
        &["for-each-ref", "--format=%(refname)"],
        "fetch advertised refs",
    )
    .unwrap();
    assert_eq!(
        refs.lines().collect::<Vec<_>>(),
        ["refs/heads/main", "refs/heads/open-child"]
    );
    assert!(has_commit(&folder, &head));
}
