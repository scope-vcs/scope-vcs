use super::*;
use scope_domain::views::ViewId;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn main_push_carrying_an_open_request_head_merges_only_that_request() {
    let (state, source, base_head) =
        super::super::push_intent_completion::published_git_fixture("request-landed").await;
    let app = router(state.clone());
    let bearer = bearer_header();
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer);

    let mut request_ids = Vec::new();
    for (name, base, submit) in [
        ("landed", base_head.as_str(), true),
        ("carried-draft", "HEAD", false),
        ("pending", base_head.as_str(), true),
    ] {
        let started = api_request(
            app.clone(),
            "POST",
            &format!("/v1/repos/{TEST_REPO_ID}/requests"),
            Some(&bearer),
            Some(&serde_json::json!({ "name": name, "view": "private" }).to_string()),
        )
        .await;
        assert_eq!(started.status(), StatusCode::OK);
        let id = response_json(started).await["request"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        run_git(Some(&source), &["checkout", "--detach", base], "pick base").unwrap();
        push_change(
            &source,
            &remote,
            &format!("refs/heads/{name}"),
            &format!("{name}.txt"),
            "request work\n",
            name,
        )
        .unwrap();
        if submit {
            let submitted = api_request(
                app.clone(),
                "POST",
                &format!("/v1/repos/{TEST_REPO_ID}/requests/{id}/submit"),
                Some(&bearer),
                Some("{}"),
            )
            .await;
            assert_eq!(submitted.status(), StatusCode::OK);
        }
        if name == "carried-draft" {
            run_git(
                Some(&source),
                &["branch", "carried", "HEAD"],
                "keep carried",
            )
            .unwrap();
        }
        request_ids.push(id);
    }

    run_git(
        Some(&source),
        &["checkout", "--detach", &base_head],
        "return to main",
    )
    .unwrap();
    run_git(
        Some(&source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "merge",
            "--no-ff",
            "--no-edit",
            "carried",
        ],
        "merge request work elsewhere",
    )
    .unwrap();
    let main_oid = git_head_oid(&source);
    let mut changes = state.repo_events.subscribe(TEST_REPO_ID);
    configure_push_intent_header(&state, &source, &remote, &test_owner_id()).await;
    run_git(
        Some(&source),
        &[
            "push",
            &remote,
            &format!("HEAD:refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "push main carrying the request",
    )
    .unwrap();

    let mut merged_refreshes = Vec::new();
    while let Ok(event) = changes.try_recv() {
        if event.kind
            == (crate::repo_events::RepoChangeKind::RepositoryChanged {
                reason: "request-merged".into(),
            })
        {
            merged_refreshes.push(event);
        }
    }
    assert_eq!(merged_refreshes.len(), 1);
    assert_eq!(
        merged_refreshes[0].version,
        crate::repo_events::REQUEST_SUMMARY_REFRESH_VERSION
    );

    let mut states = Vec::new();
    for id in &request_ids {
        let request = state
            .metadata
            .requests()
            .request_by_id(id)
            .await
            .unwrap()
            .unwrap();
        states.push((request.state(), request.merged_main_oid));
    }
    assert_eq!(
        states,
        vec![
            (RequestState::Merged, Some(main_oid)),
            (RequestState::Draft, None),
            (RequestState::Open, None),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn landed_request_is_complete_when_main_push_persistence_returns() {
    for visibility in [ViewId::public(), ViewId::private()] {
        let (state, source, base_head) =
            super::super::push_intent_completion::published_git_fixture("atomic-landed").await;
        state
            .metadata
            .requests()
            .start_request(StartRequestInput {
                id: REQUEST_ID.into(),
                repo_id: TEST_REPO_ID.into(),
                name: REQUEST_NAME.into(),
                author_user_id: test_owner_id(),
                author_role: RequestActorRole::Owner,
                view: ViewId::private(),
                base_main_oid: base_head,
                title: Some("Atomic landed request".into()),
                event_id: "event_atomic_landed_started".into(),
                now_unix: unix_now(),
            })
            .await
            .unwrap();
        let (origin, _server) = spawn_test_server(&state).await;
        let remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
        configure_bearer_header(&source, &remote, &bearer_header());
        push_change(
            &source,
            &remote,
            REQUEST_REF,
            "landed.txt",
            "landed\n",
            "landed",
        )
        .unwrap();
        state
            .metadata
            .requests()
            .submit_request(SubmitRequestCommand {
                request_id: REQUEST_ID.into(),
                actor_user_id: test_owner_id(),
                event_id: "event_atomic_landed_submitted".into(),
                now_unix: unix_now(),
            })
            .await
            .unwrap();
        let staging = clone_test_repo(&source, "atomic-landed-staging", true);
        let mut prepared = reviewed_update_from_staging_repo(
            &state,
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            &staging,
            &test_owner_id(),
            repo_config(visibility),
            ReviewedUpdateMode::ReadyPush,
        )
        .await
        .unwrap();
        prepared.base_git_frontier = Some(Some(
            find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
                .await
                .unwrap()
                .git_head
                .unwrap()
                .frontier(),
        ));
        let main_oid = prepared.head_oid.clone();
        let persisted = crate::use_cases::git_receive::main_push::persist_main_push(
            &state,
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            prepared,
            &test_owner_id(),
            &test_repo_incarnation(),
            vec![scope_postgres::db::LandedRequestCandidate {
                request_id: REQUEST_ID.into(),
                head_oid: main_oid.clone(),
            }],
        )
        .await
        .unwrap();
        let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap();
        assert_eq!(repo.git_head.unwrap().head_oid, main_oid);
        let request = stored_request(&state, REQUEST_ID).await;
        assert_eq!(persisted.completed_landed_requests, 1);
        assert_eq!(request.state(), RequestState::Merged);
        assert_eq!(request.merged_main_oid.as_deref(), Some(main_oid.as_str()));
        persisted.write_lease.release().await;
    }
}
