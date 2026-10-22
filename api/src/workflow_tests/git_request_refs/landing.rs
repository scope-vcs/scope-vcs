use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_merges_of_disjoint_requests_all_land() {
    let (state, source, base_head) =
        super::super::push_intent_completion::published_git_fixture("concurrent-landing").await;
    let app = router(state.clone());
    let bearer = bearer_header();
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer);

    let mut request_ids = Vec::new();
    for name in ["left", "middle", "right"] {
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
        run_git(
            Some(&source),
            &["checkout", "--detach", &base_head],
            "pick base",
        )
        .unwrap();
        push_change(
            &source,
            &remote,
            &format!("refs/heads/{name}"),
            &format!("{name}.txt"),
            "landed work\n",
            name,
        )
        .unwrap();
        submit_request(app.clone(), &id, &bearer).await;
        request_ids.push(id);
    }

    let merges = request_ids.iter().map(|id| {
        let app = app.clone();
        let bearer = bearer.clone();
        let route =
            scope_api_contract::routes::repo_request_merge(TEST_REPO_OWNER, TEST_REPO_NAME, id);
        async move { api_request(app, "POST", &route, Some(&bearer), None).await }
    });
    for merged in futures_util::future::join_all(merges).await {
        assert_eq!(
            expect_json(merged, StatusCode::OK).await["request"]["state"],
            "Merged"
        );
    }

    for name in ["left", "middle", "right"] {
        assert_eq!(
            live_file_content(&state, &format!("/{name}.txt"))
                .await
                .as_deref(),
            Some("landed work\n")
        );
    }
}
