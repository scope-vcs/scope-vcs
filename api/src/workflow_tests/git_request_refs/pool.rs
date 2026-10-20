use super::*;
use crate::git::request_refs::{RequestObjectRepos, request_object_repos};
use crate::use_cases::git_receive::{
    RequestRefSeeding,
    request_ref::{RequestPusher, RequestStagingKind, prepare_request_staging_repo},
};
use std::collections::BTreeSet;

async fn prepare_owner_push(state: &AppState, request_ref: &str) -> PathBuf {
    prepare_push(state, &test_owner_id(), request_ref).await
}

async fn prepare_push(state: &AppState, actor_user_id: &str, request_ref: &str) -> PathBuf {
    prepare_request_staging_repo(
        state,
        &test_repo_incarnation(),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        RequestPusher {
            actor_user_id,
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
    let request_id = start_request(&state, "pooled").await;
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
    assert_eq!(
        pooled_heads(&state, &request_id).await,
        BTreeSet::from([
            submitted_head.clone(),
            first_head.clone(),
            git_head_oid(&source)
        ])
    );
    let replica_objects = object_repos(&replica, &test_owner_id()).await.shared;
    assert!(has_commit(&replica_objects, &submitted_head));
    assert!(has_commit(&replica_objects, &first_head));
    assert!(
        !run_git_output(
            Some(&replica_objects),
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                "refs/scope/seeded/pooled"
            ],
            "check for a bundle restore",
        )
        .unwrap()
        .status
        .success()
    );
    assert!(
        replica
            .repository_engine
            .delete_repository_cache(&test_repo_incarnation())
            .unwrap()
    );
    let rematerialized = prepare_owner_push(&replica, request_ref).await;
    run_git(
        Some(&rematerialized),
        &["rev-list", "--objects", &git_head_oid(&source)],
        "read request history after canonical cache eviction",
    )
    .unwrap();
    fs::remove_dir_all(rematerialized).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replica_behind_by_a_few_revisions_of_a_long_request_catches_up_from_the_pool() {
    let (state, main_source, _) =
        super::super::push_intent_completion::published_git_fixture("request-pool-long").await;
    let replica = state.replica_for_tests();
    let bearer = bearer_header();
    start_request(&state, "long").await;
    let request_ref = "refs/heads/long";
    let (first_origin, _first_server) = spawn_test_server(&state).await;
    let (second_origin, _second_server) = spawn_test_server(&replica).await;
    let first_remote = format!("{first_origin}/git/private/{TEST_REPO_ID}");
    let second_remote = format!("{second_origin}/git/private/{TEST_REPO_ID}");
    let source = clone_test_repo(&main_source, "request-pool-long-checkout", false);
    configure_bearer_header(&source, &first_remote, &bearer);
    configure_bearer_header(&source, &second_remote, &bearer);
    run_git(
        Some(&source),
        &["checkout", "-b", "long"],
        "create request branch",
    )
    .unwrap();
    let push = |remote: &str, revision: usize| {
        push_change(
            &source,
            remote,
            request_ref,
            "long.txt",
            &format!("revision {revision}\n"),
            &format!("revision {revision}"),
        )
        .unwrap();
        git_head_oid(&source)
    };
    for revision in 0..20 {
        push(&first_remote, revision);
    }
    let restored_head = push(&second_remote, 20);
    let behind_head = push(&first_remote, 21);
    push(&second_remote, 22);

    let replica_objects = object_repos(&replica, &test_owner_id()).await.shared;
    assert!(has_commit(&replica_objects, &behind_head));
    assert_eq!(
        git_stdout_text(
            &replica_objects,
            &["rev-parse", "refs/scope/seeded/long"],
            "read the replica's bundle restore",
        )
        .unwrap()
        .trim(),
        git_stdout_text(
            &source,
            &["rev-parse", &format!("{restored_head}^")],
            "read the restored head"
        )
        .unwrap()
        .trim()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_push_restores_another_requests_objects_only_when_its_pack_needs_them() {
    let (state, main_source, _) =
        super::super::push_intent_completion::published_git_fixture("request-pool-deferred").await;
    let replica = state.replica_for_tests();
    let bearer = bearer_header();
    start_request(&state, "base-work").await;
    let follow_up_id = start_request(&state, "follow-up").await;
    let (first_origin, _first_server) = spawn_test_server(&state).await;
    let (second_origin, _second_server) = spawn_test_server(&replica).await;
    let first_remote = format!("{first_origin}/git/private/{TEST_REPO_ID}");
    let second_remote = format!("{second_origin}/git/private/{TEST_REPO_ID}");
    let source = clone_test_repo(&main_source, "request-pool-deferred-checkout", false);
    configure_bearer_header(&source, &first_remote, &bearer);
    configure_bearer_header(&source, &second_remote, &bearer);
    run_git(
        Some(&source),
        &["checkout", "-b", "base-work"],
        "create base request branch",
    )
    .unwrap();
    push_change(
        &source,
        &first_remote,
        "refs/heads/base-work",
        "base.txt",
        "base request\n",
        "base request revision",
    )
    .unwrap();
    let base_head = git_head_oid(&source);

    let unrelated = prepare_owner_push(&replica, "refs/heads/follow-up").await;
    assert!(!has_commit(&unrelated, &base_head));
    fs::remove_dir_all(unrelated).unwrap();
    push_change(
        &source,
        &second_remote,
        "refs/heads/follow-up",
        "follow-up.txt",
        "built on the base request\n",
        "follow-up revision",
    )
    .unwrap();

    assert_eq!(
        stored_request(&state, &follow_up_id).await.head_oid,
        git_head_oid(&source)
    );
    assert!(has_commit(
        &object_repos(&replica, &test_owner_id()).await.shared,
        &base_head
    ));
}

async fn start_request(state: &AppState, name: &str) -> String {
    let started = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer_header()),
        Some(&format!(r#"{{"name":"{name}","view":"private"}}"#)),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    response_json(started).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn pooled_heads(state: &AppState, request_id: &str) -> BTreeSet<String> {
    state
        .metadata
        .repositories()
        .git_pool_packs_for_requests(TEST_REPO_ID, &[request_id.to_string()])
        .await
        .unwrap()
        .into_iter()
        .map(|listed| listed.head_oid)
        .collect()
}

async fn object_repos(state: &AppState, author_id: &str) -> RequestObjectRepos {
    request_object_repos(
        state,
        &test_repo_incarnation(),
        &find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .repo_config
            .object_generation(),
        author_id,
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
async fn a_draft_revision_is_pooled_only_for_its_author() {
    let (state, _main_source, _) =
        super::super::push_intent_completion::published_git_fixture("request-pool-draft").await;
    drain_outbox(&state, "request-pool-draft-public-projection").await;
    let bearer = bearer_header();
    let started = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer),
        Some(r#"{"name":"drafted","view":"public"}"#),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    let draft_id = response_json(started).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    let source = TempGitRepo(unique_test_path("request-pool-draft-checkout"));
    run_git(
        None,
        &[
            "clone",
            &format!("{origin}/git/public/{TEST_REPO_ID}"),
            source.to_str().unwrap(),
        ],
        "clone public main for a public draft",
    )
    .unwrap();
    configure_bearer_header(&source, &remote, &bearer);
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

    let repos = object_repos(&state, &test_owner_id()).await;

    assert!(pooled_heads(&state, &draft_id).await.contains(&draft_head));
    let author_staging = prepare_owner_push(&state, request_ref).await;
    assert!(has_commit(&author_staging, &draft_head));
    let _ = fs::remove_dir_all(author_staging);
    assert!(has_commit(&repos.authored, &draft_head));
    assert!(!has_commit(&repos.shared, &draft_head));
    insert_member_user(&state).await;
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            for member in &mut repo.collaboration.members {
                member.permissions.can_push = true;
            }
        })
        .await
        .unwrap();
    let member_request = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL)),
        Some(r#"{"name":"member-work","view":"private"}"#),
    )
    .await;
    assert_eq!(member_request.status(), StatusCode::OK);
    let member_staging = prepare_push(&state, &member_user_id(), "refs/heads/member-work").await;
    assert!(!has_commit(&member_staging, &draft_head));
    let _ = fs::remove_dir_all(member_staging);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_whose_view_was_removed_cannot_be_grafted_from_the_pool() {
    assert_removed_view_pool_isolation(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_push_prepared_before_view_removal_cannot_seed_the_new_generation() {
    assert_removed_view_pool_isolation(true).await;
}

async fn assert_removed_view_pool_isolation(prepare_before_removal: bool) {
    let label = if prepare_before_removal {
        "removed-view-pool-delayed"
    } else {
        "removed-view-pool"
    };
    let (state, _main_source) = test_state_with_mergeable_request(label).await;
    let (source, public_remote, _server, _) =
        request_checkout(&state, &format!("{label}-checkout")).await;
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
    if prepare_before_removal {
        configure_bearer_header(&_main_source, &private_remote, &bearer_header());
        push_change(
            &_main_source,
            &private_remote,
            "refs/heads/other",
            "other.txt",
            "authorized request\n",
            "authorized request revision",
        )
        .unwrap();
        submit_request(router(state.clone()), &other_id, &bearer_header()).await;
    }
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
        pooled_heads(&state, REQUEST_ID)
            .await
            .contains(&hidden_head)
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
    let preparation = if prepare_before_removal {
        Some(
            git_receive_use_case::prepare(
                &replica,
                TEST_REPO_OWNER,
                TEST_REPO_NAME,
                ReceivePackAccess::RequestContributor {
                    author_id: test_owner_id(),
                    incarnation: test_repo_incarnation(),
                    view: ViewId::private(),
                },
                RequestRefSeeding::Pushed(&["refs/heads/other".to_string()]),
            )
            .await
            .unwrap(),
        )
    } else {
        None
    };
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
    let packed = if prepare_before_removal {
        run_git(
            Some(&_main_source),
            &[
                "fetch",
                source.to_str().unwrap(),
                "+HEAD:refs/scope-test/hidden",
            ],
            "copy the hidden head into the grafting checkout",
        )
        .unwrap();
        pack.arg("--revs");
        format!("{graft}\n^{old}\n")
    } else {
        format!("{graft}\n")
    };
    payload.extend(
        crate::git::command::git_command_output_with_timeout(
            &mut pack,
            Some(packed.into_bytes()),
            std::time::Duration::from_secs(5),
        )
        .unwrap(),
    );
    if let Some(preparation) = preparation {
        let staging = preparation.staging_repo.clone();
        let mut receive = std::process::Command::new("git");
        receive
            .args(["receive-pack", "--stateless-rpc"])
            .arg(&staging);
        let received = crate::git::command::git_command_output_with_timeout(
            &mut receive,
            Some(payload),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&received).contains("ok refs/heads/other"));
        let completion = git_receive_use_case::complete(
            &replica,
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            &staging,
            preparation,
            std::time::Duration::ZERO,
        )
        .await;
        let generation = find_repo(&replica, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .repo_config
            .object_generation();
        let current = request_object_repos(
            &replica,
            &test_repo_incarnation(),
            &generation,
            &test_owner_id(),
        )
        .unwrap();
        let retained = has_commit(&current.shared, &hidden_head)
            || has_commit(&current.authored, &hidden_head);
        let fresh = replica.replica_for_tests();
        let restored_staging = prepare_owner_push(&fresh, "refs/heads/other").await;
        let restored = has_commit(&restored_staging, &hidden_head);
        fs::remove_dir_all(restored_staging).unwrap();
        fs::remove_dir_all(staging).unwrap();
        assert!(
            completion.is_err() && !retained && !restored,
            "completion: {completion:?}; unreadable parent retained in new generation: {retained}; unreadable parent restored on fresh replica: {restored}"
        );
        return;
    }
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
