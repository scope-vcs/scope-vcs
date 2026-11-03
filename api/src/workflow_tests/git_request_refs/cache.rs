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

struct SnapshotReadFails {
    snapshot_key: String,
    readable: Arc<dyn ObjectStore>,
}

#[async_trait::async_trait]
impl ObjectStore for SnapshotReadFails {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), scope_storage::ObjectStoreError> {
        self.readable.put(key, bytes).await
    }

    async fn read_to(
        &self,
        key: &str,
        max_bytes: u64,
        output: &mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    ) -> Result<u64, scope_storage::ObjectStoreError> {
        if key == self.snapshot_key {
            return Err(scope_storage::ObjectStoreError::service_unavailable(
                "snapshot read failed for test",
            ));
        }
        self.readable.read_to(key, max_bytes, output).await
    }

    async fn delete(&self, key: &str) -> Result<(), scope_storage::ObjectStoreError> {
        self.readable.delete(key).await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_omits_an_unreadable_pack_and_snapshot_then_restores_the_legacy_snapshot_once() {
    let state = test_state_with_request().await;
    let (source, _remote, _server, head) =
        request_checkout(&state, "read-view-unreadable-snapshot").await;
    let mut legacy = stored_request(&state, REQUEST_ID).await;
    run_git(
        Some(&source),
        &["update-ref", "refs/heads/legacy", &head],
        "seed legacy snapshot ref",
    )
    .unwrap();
    let (snapshot, bytes) =
        crate::git::import::git_snapshot_from_ref(&source, "refs/heads/legacy", None).unwrap();
    state
        .object_store
        .put(&scope_storage::object_key(&snapshot), bytes)
        .await
        .unwrap();
    legacy.name = "legacy".to_string();
    legacy.git_snapshot = Some(snapshot.clone());
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
    let mut state = state.replica_for_tests();
    let readable = state.object_store.clone();
    state.object_store = Arc::new(SnapshotReadFails {
        snapshot_key: scope_storage::object_key(&snapshot),
        readable: readable.clone(),
    });
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

    state.object_store = readable;
    let recovered = read(state.clone()).await;
    assert_eq!(ref_head(recovered.as_ref(), "refs/heads/legacy"), head);
    state.object_store = Arc::new(SnapshotReadFails {
        snapshot_key: scope_storage::object_key(&snapshot),
        readable: state.object_store.clone(),
    });
    let reused = read(state.clone()).await;
    assert_eq!(ref_head(&reused, "refs/heads/legacy"), head);
}
