use super::*;

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
            Some(&serde_json::json!({ "name": name, "audience": "Private" }).to_string()),
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
