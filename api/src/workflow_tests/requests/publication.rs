use super::*;

#[tokio::test]
async fn public_request_reads_remain_available_before_projection_outbox_catches_up() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo("request-publication-read");
    fs::write(source.join("README.md"), "before\n").unwrap();
    fs::create_dir_all(source.join("internal")).unwrap();
    fs::write(source.join("internal/notes.md"), "private\n").unwrap();
    run_git(Some(&source), &["add", "."], "stage publication fixture").unwrap();
    commit_all(&source, "initial public and private content");
    let mut config = repo_config(Visibility::Public);
    config
        .visibility
        .rules
        .push(scope_domain::repo_config::RepoConfigVisibilityRule {
            path: "/internal".into(),
            visibility: ConfigVisibility::Private,
        });
    let first = clone_test_repo(&source, "request-publication-first", true);
    apply_first_push_from_staging_repo(&state, &first, config.clone()).await;

    // Fixture creation drains existing jobs; all requests must precede the push under test.
    create_public_request(
        &state,
        "req_publication_public",
        test_owner_id(),
        REQUEST_HEAD,
    )
    .await;
    create_owner_request(&state, "req_publication_private", REQUEST_HEAD).await;
    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_publication_public", |request| {
            request.submitted_at_unix = Some(4);
            request.updated_at_unix = 4;
        })
        .await
        .unwrap();

    fs::write(source.join("README.md"), "after\n").unwrap();
    run_git(Some(&source), &["add", "."], "stage publication update").unwrap();
    commit_all(&source, "update public README");
    let second = clone_test_repo(&source, "request-publication-second", true);
    let before = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let mut update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &second,
        &test_owner_id(),
        config,
    )
    .await
    .unwrap();
    update.base_git_frontier = Some(Some(before.git_head.unwrap().frontier()));
    let accepted = persist_test_update(&state, update).await.unwrap();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let public_projection = scope_domain::projection::project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        scope_domain::projection::ProjectionViewKey::Public,
    );
    let expected_public_head = scope_git::projection_head_oid(&public_projection)
        .unwrap()
        .unwrap();
    assert_ne!(expected_public_head, accepted.head_oid);

    // No worker/outbox runs after publication. Reads must use the current audience's head.
    let app = router(state);
    for uri in [
        "/v1/repos/owner/repo/requests",
        "/v1/repos/owner/repo/requests/queue?section=active",
        "/v1/repos/owner/repo/requests/req_publication_public",
    ] {
        let response = api_request(app.clone(), "GET", uri, None, None).await;
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let body = response_json(response).await;
        let request = if body.get("requests").is_some() {
            assert_eq!(request_ids(&body), ["req_publication_public"]);
            &body["requests"][0]["request"]
        } else {
            &body["request"]
        };
        assert_eq!(
            request["mergeability"]["current_main_oid"],
            expected_public_head
        );
    }
    let hidden = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests/req_publication_private",
        None,
        None,
    )
    .await;
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
    let private = api_request(
        app,
        "GET",
        "/v1/repos/owner/repo/requests/req_publication_private",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(private.status(), StatusCode::OK);
    assert_eq!(
        response_json(private).await["request"]["mergeability"]["current_main_oid"],
        accepted.head_oid,
    );
}
