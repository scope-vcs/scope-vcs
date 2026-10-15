use super::*;

#[tokio::test]
async fn metadata_updates_persist_in_repository_and_public_owner_summaries() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let before = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let mut events = state.repo_events.subscribe(&before.record.id);
    let rejected = api_request(
        router(state.clone()),
        "PATCH",
        "/v1/repos/owner/repo/metadata",
        Some(&bearer_header()),
        Some(
            &serde_json::json!({
                "description": "Must not save", "website_url": "javascript:alert(1)"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .record
            .description,
        None
    );
    assert!(events.try_recv().is_err());
    let response = api_request(
        router(state.clone()),
        "PATCH",
        "/v1/repos/owner/repo/metadata",
        Some(&bearer_header()),
        Some(
            &serde_json::json!({
                "description": "  A focused project  ", "website_url": " https://example.com/docs "
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["description"], "A focused project");
    assert_eq!(body["website_url"], "https://example.com/docs");
    assert_eq!(body["change_version"], before.record.change_version + 1);
    let event = events.try_recv().unwrap();
    assert_eq!(
        event.kind,
        crate::repo_events::RepoChangeKind::RepositoryChanged {
            reason: "metadata-updated".into()
        }
    );
    let saved = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(
        saved.record.description.as_deref(),
        Some("A focused project")
    );
    assert_eq!(
        saved.record.website_url.as_deref(),
        Some("https://example.com/docs")
    );
    let public = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo",
        None,
        None,
    )
    .await;
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(
        response_json(public).await["description"],
        "A focused project"
    );
    let owner = api_request(
        router(state.clone()),
        "GET",
        "/v1/users/owner/repos",
        None,
        None,
    )
    .await;
    assert_eq!(owner.status(), StatusCode::OK);
    assert_eq!(
        response_json(owner).await["repositories"][0]["website_url"],
        "https://example.com/docs"
    );
    let unchanged = api_request(
        router(state.clone()),
        "PATCH",
        "/v1/repos/owner/repo/metadata",
        Some(&bearer_header()),
        Some(
            &serde_json::json!({
                "description": "A focused project", "website_url": "https://example.com/docs"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(unchanged.status(), StatusCode::OK);
    assert_eq!(
        response_json(unchanged).await["change_version"],
        before.record.change_version + 1
    );
    assert!(events.try_recv().is_err());
    let cleared = api_request(
        router(state),
        "PATCH",
        "/v1/repos/owner/repo/metadata",
        Some(&bearer_header()),
        Some(
            &serde_json::json!({
                "description": "   ", "website_url": null
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(cleared.status(), StatusCode::OK);
    let cleared = response_json(cleared).await;
    assert!(cleared["description"].is_null());
    assert!(cleared["website_url"].is_null());
}

#[tokio::test]
async fn metadata_editing_requires_membership_but_no_member_capabilities() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let unreadable = test_state_with_repo();
    cache_test_jwks(&unreadable);
    let update = serde_json::json!({"description": "Member edit", "website_url": null});
    let outsider = bearer_header_for("user_other", "other@example.com");
    for (target, bearer, status) in [
        (&state, None, StatusCode::UNAUTHORIZED),
        (&state, Some(outsider.as_str()), StatusCode::FORBIDDEN),
        (&unreadable, Some(outsider.as_str()), StatusCode::NOT_FOUND),
    ] {
        let response = api_request(
            router(target.clone()),
            "PATCH",
            "/v1/repos/owner/repo/metadata",
            bearer,
            Some(&update.to_string()),
        )
        .await;
        assert_eq!(response.status(), status);
    }
    let member_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "user_member");
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(member_id.clone(), "member", "member@example.com"))
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.collaboration.members.push(test_repository_member(
                TEST_REPO_ID,
                member_id,
                RepositoryMemberPermissions::default(),
            ));
        })
        .await
        .unwrap();
    let member = api_request(
        router(state),
        "PATCH",
        "/v1/repos/owner/repo/metadata",
        Some(&bearer_header_for("user_member", "member@example.com")),
        Some(&update.to_string()),
    )
    .await;
    assert_eq!(member.status(), StatusCode::OK);
    let body = response_json(member).await;
    assert_eq!(body["description"], "Member edit");
    assert_eq!(body["access"]["actor"], "Member");
}
