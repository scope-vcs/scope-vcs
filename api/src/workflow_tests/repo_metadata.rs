use super::*;

#[tokio::test]
async fn metadata_updates_persist_in_repository_and_public_owner_summaries() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let before = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let mut events = state.repo_events.subscribe(&before.record.id);
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
    let update = serde_json::json!({"description": "Member edit", "website_url": null});
    let anonymous = api_request(
        router(state.clone()),
        "PATCH",
        "/v1/repos/owner/repo/metadata",
        None,
        Some(&update.clone().to_string()),
    )
    .await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let outsider = api_request(
        router(state.clone()),
        "PATCH",
        "/v1/repos/owner/repo/metadata",
        Some(&bearer_header_for("user_other", "other@example.com")),
        Some(&update.clone().to_string()),
    )
    .await;
    assert_eq!(outsider.status(), StatusCode::FORBIDDEN);
    let invite = api_request(
        router(state.clone()),
        "POST",
        "/v1/repos/owner/repo/invites",
        Some(&bearer_header()),
        Some(
            &serde_json::json!({
                "email": "member@example.com", "permissions": RepositoryMemberPermissions::default()
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(invite.status(), StatusCode::OK);
    let invite = response_json(invite).await;
    let token = invite["invite_url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap();
    let accepted = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repository-invites/{token}/accept"),
        Some(&bearer_header_for("user_member", "member@example.com")),
        None,
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
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

#[tokio::test]
async fn metadata_validation_rejects_unsafe_urls_without_saving_description() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    for website in [
        "javascript:alert(1)",
        "/relative",
        "https://user:password@example.com",
    ] {
        let response = api_request(
            router(state.clone()),
            "PATCH",
            "/v1/repos/owner/repo/metadata",
            Some(&bearer_header()),
            Some(
                &serde_json::json!({
                    "description": "Must not save", "website_url": website
                })
                .to_string(),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let invalid_description = api_request(
        router(state.clone()),
        "PATCH",
        "/v1/repos/owner/repo/metadata",
        Some(&bearer_header()),
        Some(
            &serde_json::json!({
                "description": "x".repeat(161), "website_url": null
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(invalid_description.status(), StatusCode::BAD_REQUEST);
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    assert_eq!(repo.record.description, None);
    assert_eq!(repo.record.website_url, None);
}

#[tokio::test]
async fn metadata_updates_hide_repositories_the_viewer_cannot_read() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let response = api_request(
        router(state),
        "PATCH",
        "/v1/repos/owner/repo/metadata",
        Some(&bearer_header_for("user_other", "other@example.com")),
        Some(&serde_json::json!({"description": "Must not save", "website_url": null}).to_string()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
