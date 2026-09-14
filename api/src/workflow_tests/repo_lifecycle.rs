use super::*;
use crate::repo_events::RepoChangeReason;

#[tokio::test]
async fn create_repo_route_creates_user_and_lists_repo() {
    let state = test_state_with_jwks();
    let response = api_request(
        router(state.clone()),
        "POST",
        "/v1/repos",
        Some(&bearer_header()),
        Some(r#"{"name":"Scope_App"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["repo"]["id"], "owner/scope_app");
    assert_eq!(body["repo"]["access"]["actor"], "Owner");
    assert_eq!(
        body["repo"]["git_remote_url"],
        "http://localhost:8080/git/permissioned/owner/scope_app"
    );
    assert_eq!(
        body["init"]["git_remote_url"],
        "http://localhost:8080/git/permissioned/owner/scope_app"
    );
    let secret = body["init"]["token"]["secret"].as_str().unwrap();
    assert!(secret.starts_with("scope_fp_"));
    let push_secret = body["init"]["push_token"]["secret"].as_str().unwrap();
    assert!(push_secret.starts_with("scope_git_"));

    let duplicate = api_request(
        router(state.clone()),
        "POST",
        "/v1/repos",
        Some(&bearer_header()),
        Some(r#"{"name":"Scope_App"}"#),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    let duplicate: scope_api_contract::ErrorResponse =
        serde_json::from_value(response_json(duplicate).await).unwrap();
    assert_eq!(duplicate.code, scope_api_contract::ErrorCode::Conflict);
    assert!(duplicate.instruction.unwrap().contains("scope init --name"));

    let response = api_request(
        router(state.clone()),
        "GET",
        "/v1/users/owner/repos",
        Some(&bearer_header()),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["handle"], "owner");
    assert_eq!(body["repositories"][0]["id"], "owner/scope_app");
}

#[tokio::test]
async fn invite_acceptance_returns_member_access() {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    let (analytics, recording) = scope_product_analytics::ProductAnalytics::recording();
    state.product_analytics = analytics;
    let invited_email = "invitee@example.com";
    let create_response = api_request(
        router(state.clone()),
        "POST",
        "/v1/repos/owner/repo/invites",
        Some(&bearer_header()),
        Some(
            serde_json::json!({
                "email": invited_email,
                "permissions": RepositoryMemberPermissions::default(),
            })
            .to_string(),
        )
        .as_deref(),
    )
    .await;
    assert_eq!(create_response.status(), StatusCode::OK);
    let create_body = response_json(create_response).await;
    let invite_url = create_body["invite_url"].as_str().unwrap();
    let token = invite_url.rsplit('/').next().unwrap();

    let accept_response = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repository-invites/{token}/accept"),
        Some(&bearer_header_for("user_invitee", invited_email)),
        None,
    )
    .await;

    assert_eq!(accept_response.status(), StatusCode::OK);
    let body = response_json(accept_response).await;
    assert_eq!(body["repo"]["access"]["actor"], "Member");
    assert_eq!(
        recording.event_names(),
        ["account:user_create", "repository:invite_accept"]
    );
    assert_eq!(
        recording.property(1, "repository_id"),
        Some(serde_json::Value::String("repoi_workflow_test".into()))
    );
    assert_eq!(
        recording.property(1, "actor_role"),
        Some(serde_json::Value::String("member".into()))
    );

    let repeated = api_request(
        router(state),
        "POST",
        &format!("/v1/repository-invites/{token}/accept"),
        Some(&bearer_header_for("user_invitee", invited_email)),
        None,
    )
    .await;
    assert_eq!(repeated.status(), StatusCode::CONFLICT);
    assert_eq!(
        recording.event_names(),
        ["account:user_create", "repository:invite_accept"]
    );
}

#[tokio::test]
async fn owner_can_revoke_pending_invite_before_acceptance() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let token = "revoked-invite-token";
    let invited_email = "invitee@example.com";
    let now = unix_now();
    let invite = RepositoryInvite {
        id: "invite_revoke".into(),
        repo_id: TEST_REPO_ID.into(),
        invited_email: invited_email.into(),
        invited_email_normalized:
            scope_domain::repository::collaboration::normalize_repository_invite_email(
                invited_email,
            ),
        permissions: RepositoryMemberPermissions::default(),
        invited_by_user_id: test_owner_id(),
        state: RepositoryInviteState::Pending,
        token_hash: token_hash(token),
        created_at_unix: now,
        updated_at_unix: now,
        expires_at_unix: now + 600,
        accepted_by_user_id: None,
        accepted_at_unix: None,
        revoked_at_unix: None,
    };
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, move |repo| repo.invitations.push(invite))
        .await
        .unwrap();

    let revoke_response = api_request(
        router(state.clone()),
        "DELETE",
        "/v1/repos/owner/repo/invites/invite_revoke",
        Some(&bearer_header()),
        None,
    )
    .await;

    assert_eq!(revoke_response.status(), StatusCode::OK);
    let body = response_json(revoke_response).await;
    assert_eq!(body["state"], "Revoked");

    let accept_response = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repository-invites/{token}/accept"),
        Some(&bearer_header_for("user_invitee", invited_email)),
        None,
    )
    .await;

    assert_eq!(accept_response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn collaboration_publication_keeps_the_committed_invite_result_and_version() {
    let state = test_state_with_repo();
    let owner = state
        .metadata
        .repositories()
        .user(&test_owner_id())
        .await
        .unwrap();
    let initial_version = state
        .metadata
        .repositories()
        .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .unwrap()
        .record
        .change_version;
    let (secret, token_hash) = crate::auth::tokens::generate_repository_invite_token().unwrap();
    let invite = state
        .metadata
        .repositories()
        .create_repository_invite(
            scope_postgres::db::CreateRepositoryInviteMutation {
                owner: "owner".to_string(),
                name: "repo".to_string(),
                owner_user: owner,
                invited_email: "later@example.com".to_string(),
                permissions: Default::default(),
                invite_id: "invite_version".to_string(),
                token_hash,
                now_unix: unix_now(),
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    assert_eq!(invite.change_version, initial_version + 1);
    let committed_version = invite.change_version;
    // A later committed mutation must not overwrite this operation's version
    // while its response is still waiting for notification publication.
    let revoked = state
        .metadata
        .repositories()
        .revoke_repository_invite(
            "owner",
            "repo",
            &test_owner_id(),
            &invite.value.id,
            unix_now(),
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    assert!(revoked.change_version > committed_version);
    let expected_url = format!("https://app.example.com/invites/{secret}");
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let response = crate::use_cases::repository_collaboration::publish_committed_mutation(
        &state,
        crate::use_cases::repository_collaboration::map_committed_mutation(invite, |invite| {
            (invite.id, expected_url.clone())
        }),
        RepoChangeReason::InviteUpdated,
    )
    .await;
    assert_eq!(response, ("invite_version".to_string(), expected_url));
    let event = events.recv().await.unwrap();
    assert_eq!(event.version, committed_version);
    assert_eq!(
        event.incarnation_id,
        state
            .metadata
            .repositories()
            .repository(TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap()
            .unwrap()
            .record
            .incarnation_id
    );
}
