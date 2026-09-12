use super::*;

#[tokio::test]
async fn history_resolves_handles_in_pages_and_details_without_changing_stored_authors() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let repo = paged_history_repo(&state, 2);
    let source_id = repo.graph.commits.last().unwrap().id.clone();
    replace_test_repo(&state, repo).await;
    assert_ne!(test_owner_id(), TEST_REPO_OWNER);

    for audience in ["public", "private"] {
        let page = api_request(
            router(state.clone()),
            "GET",
            &format!("/v1/repos/owner/repo/history?feed=all&audience={audience}"),
            (audience == "private").then(bearer_header).as_deref(),
            None,
        )
        .await;
        assert_eq!(page.status(), StatusCode::OK);
        let page = response_json(page).await;
        for entry in page["entries"].as_array().unwrap() {
            assert_eq!(entry["author"], TEST_REPO_OWNER);
        }
        let detail = api_request(
            router(state.clone()),
            "GET",
            &format!("/v1/repos/owner/repo/history/{source_id}?audience={audience}"),
            (audience == "private").then(bearer_header).as_deref(),
            None,
        )
        .await;
        assert_eq!(detail.status(), StatusCode::OK);
        assert_eq!(response_json(detail).await["author"], TEST_REPO_OWNER);
    }

    let stored = state
        .metadata
        .repositories()
        .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored
            .graph
            .commits
            .iter()
            .all(|commit| commit.author_id == test_owner_id())
    );
}

#[tokio::test]
async fn public_history_keeps_partial_update_authors_hidden() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    replace_test_repo(
        &state,
        history_repo(
            vec![logical_commit(
                "mixed-update",
                "Private metadata",
                vec![
                    history_change(
                        "/README.md",
                        Visibility::Public,
                        None,
                        Some(source_blob(&state, "public")),
                    ),
                    history_change(
                        "/secret.txt",
                        Visibility::Private,
                        None,
                        Some(source_blob(&state, "secret")),
                    ),
                ],
            )],
            Some("/README.md"),
        ),
    )
    .await;

    let page = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/history?audience=public",
        None,
        None,
    )
    .await;
    assert_eq!(page.status(), StatusCode::OK);
    let page = response_json(page).await;
    assert!(page["entries"][0]["author"].is_null());
    let source_id = page["entries"][0]["source_id"].as_str().unwrap();
    let detail = api_request(
        router(state),
        "GET",
        &format!("/v1/repos/owner/repo/history/{source_id}?audience=public"),
        None,
        None,
    )
    .await;
    assert_eq!(detail.status(), StatusCode::OK);
    assert!(response_json(detail).await["author"].is_null());
}
