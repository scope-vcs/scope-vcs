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
        "draft.txt",
        "before submission\n",
        "draft revision",
    )
    .unwrap();
    let submitted_head = git_head_oid(&source);
    submit_request(router(state.clone()), &request_id, &bearer).await;
    let _ = fs::remove_dir_all(prepare_owner_push(&replica, request_ref).await);
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
            .map(|listed| listed.pack.head_oid.as_str())
            .collect::<Vec<_>>(),
        [first_head.as_str(), git_head_oid(&source).as_str()]
    );
    let replica_objects = objects_repo(&replica).await;
    assert!(has_commit(&replica_objects, &first_head));
    assert_eq!(
        git_stdout_text(
            &replica_objects,
            &["rev-parse", "refs/scope/seeded/pooled"],
            "read the replica's bundle restore",
        )
        .unwrap()
        .trim(),
        submitted_head
    );
}

async fn objects_repo(state: &AppState) -> PathBuf {
    crate::git::request_refs::request_objects_repo(
        state,
        &test_repo_incarnation(),
        &find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .repo_config,
    )
    .unwrap()
}

fn has_commit(repo: &FsPath, oid: &str) -> bool {
    run_git_output(
        Some(repo),
        &["cat-file", "-e", &format!("{oid}^{{commit}}")],
        "check local commit",
    )
    .map(|output| output.status.success())
    .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_draft_revision_stays_out_of_shared_replica_objects() {
    let (state, main_source, _) =
        super::super::push_intent_completion::published_git_fixture("request-pool-draft").await;
    let bearer = bearer_header();
    let started = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer),
        Some(r#"{"name":"drafted","view":"private"}"#),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    let source = clone_test_repo(&main_source, "request-pool-draft-checkout", false);
    configure_bearer_header(&source, &remote, &bearer);
    run_git(
        Some(&source),
        &["checkout", "-b", "drafted"],
        "create draft branch",
    )
    .unwrap();
    let request_ref = "refs/heads/drafted";
    push_change(
        &source,
        &remote,
        request_ref,
        "draft.txt",
        "only the author sees this\n",
        "draft revision",
    )
    .unwrap();
    let draft_head = git_head_oid(&source);

    let staging = prepare_owner_push(&state, request_ref).await;

    assert!(has_commit(&staging, &draft_head));
    let _ = fs::remove_dir_all(staging);
    assert!(
        state
            .metadata
            .repositories()
            .git_pool_packs_after(TEST_REPO_ID, Some(0))
            .await
            .unwrap()
            .packs
            .is_empty()
    );
    assert!(!has_commit(&objects_repo(&state).await, &draft_head));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_whose_view_was_removed_cannot_be_grafted_from_the_pool() {
    let (state, _main_source) = test_state_with_mergeable_request("removed-view-pool").await;
    let (source, public_remote, _server, _) =
        request_checkout(&state, "removed-view-pool-checkout").await;
    submit_request(
        router(state.clone()),
        REQUEST_ID,
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    )
    .await;
    let started = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer_header()),
        Some(r#"{"name":"other","view":"private"}"#),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    let other_id = response_json(started).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let replica = state.replica_for_tests();
    fs::remove_dir_all(prepare_owner_push(&replica, "refs/heads/other").await).unwrap();
    let private_remote = public_remote.replace("/git/public/", "/git/private/");
    configure_bearer_header(&source, &private_remote, &bearer_header());
    push_change(
        &source,
        &private_remote,
        REQUEST_REF,
        "request.txt",
        "unreadable after view removal\n",
        "pooled before view removal",
    )
    .unwrap();
    let hidden_head = git_head_oid(&source);
    assert!(
        state
            .metadata
            .repositories()
            .git_pool_packs_after(TEST_REPO_ID, Some(0))
            .await
            .unwrap()
            .packs
            .iter()
            .any(|listed| listed.pack.head_oid == hidden_head)
    );
    state
        .metadata
        .requests()
        .close_request(scope_postgres::db::CloseRequestCommand {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            event_id: "close_before_view_removal".to_string(),
            now_unix: unix_now(),
        })
        .await
        .unwrap();
    for remove_public in [false, true] {
        let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap();
        let mut next = repo_config(ViewId::private());
        if remove_public {
            next.views = scope_domain::views::Views::new(
                next.views
                    .iter()
                    .filter(|view| view.id != ViewId::public())
                    .cloned()
                    .collect(),
            )
            .unwrap();
        }
        let body = push_intent_request_json_with_base(
            &repo.git_head.as_ref().unwrap().head_oid,
            repo_config_fingerprint(&repo.repo_config).unwrap(),
            next,
        );
        let response = api_request(
            router(state.clone()),
            "POST",
            &format!("/v1/repos/{TEST_REPO_ID}/push-intents"),
            Some(&bearer_header()),
            Some(&body),
        )
        .await;
        let status = response.status();
        assert_eq!(status, StatusCode::OK, "{}", response_json(response).await);
    }
    let response = api_request(
        router(replica.clone()),
        "GET",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let staging = prepare_owner_push(&replica, "refs/heads/other").await;
    assert!(!has_commit(&staging, &hidden_head));
    fs::remove_dir_all(staging).unwrap();
    let old = stored_request(&replica, &other_id).await.head_oid;
    let tree = git_stdout_text(
        &_main_source,
        &["rev-parse", "HEAD^{tree}"],
        "read authorized tree",
    )
    .unwrap();
    let raw_commit = format!(
        "tree {}\nparent {old}\nparent {hidden_head}\nauthor Review <review@example.invalid> 1700000000 +0000\ncommitter Review <review@example.invalid> 1700000000 +0000\n\ngraft an unreadable parent\n",
        tree.trim(),
    );
    let mut hash = std::process::Command::new("git");
    hash.current_dir(&_main_source)
        .args(["hash-object", "-t", "commit", "-w", "--stdin"]);
    let graft = crate::git::command::git_command_output_with_timeout(
        &mut hash,
        Some(raw_commit.into_bytes()),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    let graft = String::from_utf8(graft).unwrap().trim().to_string();
    let command = format!("{old} {graft} refs/heads/other\0report-status\n");
    let mut payload = format!("{:04x}{command}0000", command.len() + 4).into_bytes();
    let mut pack = std::process::Command::new("git");
    pack.current_dir(&_main_source)
        .args(["pack-objects", "--stdout"]);
    payload.extend(
        crate::git::command::git_command_output_with_timeout(
            &mut pack,
            Some(format!("{graft}\n").into_bytes()),
            std::time::Duration::from_secs(5),
        )
        .unwrap(),
    );
    let response = router(replica.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/git/private/{TEST_REPO_ID}/git-receive-pack"))
                .header(CONTENT_TYPE, "application/x-git-receive-pack-request")
                .header(AUTHORIZATION, bearer_header())
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_048_576).await.unwrap();
    assert!(
        String::from_utf8_lossy(&bytes).contains("ng refs/heads/other missing necessary objects"),
        "{status} {}",
        String::from_utf8_lossy(&bytes)
    );
    assert_ne!(
        stored_request(&replica, &other_id).await.head_oid,
        graft,
        "a one-object push copied an unreadable request into an authorized branch"
    );
}
