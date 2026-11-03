use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_folders_keep_draft_visibility_separate() {
    let state = test_state_with_request().await;
    insert_public_contributor(&state).await;
    let (_source, _remote, _server, draft) =
        request_checkout(&state, "fetch-draft-isolation").await;
    let author = authorization_headers(bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL));
    let contributor =
        authorization_headers(bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL));
    let author_view = fetch_folder(&state, &author, &scope_domain::views::ViewId::public()).await;
    let contributor_view =
        fetch_folder(&state, &contributor, &scope_domain::views::ViewId::public()).await;
    let repeat = fetch_folder(&state, &author, &ViewId::public()).await;
    assert_ne!(author_view.as_ref(), repeat.as_ref());
    assert_eq!(
        git_stdout_text(&author_view, &["show-ref"], "first fetch refs").unwrap(),
        git_stdout_text(&repeat, &["show-ref"], "repeat fetch refs").unwrap(),
    );
    assert_ne!(author_view.as_ref(), contributor_view.as_ref());
    assert!(
        git_stdout_text(
            author_view.as_ref(),
            &["rev-parse", "--verify", REQUEST_REF],
            "author draft ref"
        )
        .is_ok()
    );
    assert!(
        git_stdout_text(
            contributor_view.as_ref(),
            &["rev-parse", "--verify", REQUEST_REF],
            "unrelated draft ref"
        )
        .is_err()
    );
    assert!(
        run_git_output(
            Some(&author_view),
            &["cat-file", "-e", &draft],
            "author draft object"
        )
        .unwrap()
        .status
        .success()
    );
    assert!(
        !run_git_output(
            Some(&contributor_view),
            &["cat-file", "-e", &draft],
            "other author draft object"
        )
        .unwrap()
        .status
        .success()
    );
    let alternate = fs::read_to_string(contributor_view.join("objects/info/alternates")).unwrap();
    let shelf = FsPath::new(alternate.lines().next().unwrap())
        .parent()
        .unwrap();
    let shared = crate::git::request_refs::request_object_repos(
        &state,
        &test_repo_incarnation(),
        &find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .repo_config
            .object_generation(),
        &test_owner_id(),
    )
    .unwrap()
    .shared;
    let mut borrowed = fs::read_to_string(shelf.join("objects/info/alternates")).unwrap();
    borrowed.push_str(&format!("{}\n", shared.join("objects").display()));
    fs::write(shelf.join("objects/info/alternates"), borrowed).unwrap();
    let error = git_upload_pack_repo_for_request(
        &state,
        &contributor,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &ViewId::public(),
    )
    .await
    .unwrap_err();
    assert!(
        error
            .operator_diagnostic()
            .contains("unauthorized alternate")
    );
    let paths = [
        author_view.as_ref().to_path_buf(),
        contributor_view.as_ref().to_path_buf(),
        repeat.as_ref().to_path_buf(),
    ];
    drop(author_view);
    drop(contributor_view);
    drop(repeat);
    assert!(paths.iter().all(|path| !path.exists()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_omits_an_unreadable_pool_pack() {
    let state = test_state_with_request().await;
    let (_source, _remote, _server, _head) =
        request_checkout(&state, "read-view-unreadable-snapshot").await;
    let mut legacy = stored_request(&state, REQUEST_ID).await;
    legacy.name = "legacy".to_string();
    state
        .metadata
        .requests()
        .mutate_request_for_tests(REQUEST_ID, |request| {
            request.submitted_at_unix = Some(request.updated_at_unix);
            request.closed_at_unix = Some(request.updated_at_unix);
            request.closed_by_user_id = Some(test_owner_id());
        })
        .await
        .unwrap();
    legacy.id = "req_legacy".to_string();
    state
        .metadata
        .requests()
        .insert_request_for_tests(legacy.clone())
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .register_main_pool_pack_for_tests(
            TEST_REPO_ID,
            &scope_domain::repository::git::GitPoolPack {
                id: "broken-fetch-pack".to_string(),
                owner: scope_domain::repository::git::GitPoolPackOwner::Request {
                    request_id: legacy.id.clone(),
                    scope: scope_domain::repository::git::RequestPackScope::View(ViewId::public()),
                },
                head_oid: legacy.head_oid.clone(),
                prerequisites: Vec::new(),
                storage: scope_domain::repository::git::GitPoolPackStorage::Inline {
                    sealed: vec![0],
                },
            },
        )
        .await
        .unwrap();
    let state = state.replica_for_tests();
    let author = &authorization_headers(bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL));
    let read =
        |state: AppState| async move { fetch_folder(&state, author, &ViewId::public()).await };

    let degraded = read(state.clone()).await;
    assert!(
        git_stdout_text(
            degraded.as_ref(),
            &["rev-parse", "--verify", "refs/heads/legacy"],
            "omitted request ref"
        )
        .is_err()
    );
}
