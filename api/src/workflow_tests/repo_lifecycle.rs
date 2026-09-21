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
                email_id: "email_version".to_string(),
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
            &invite.value.0.id,
            unix_now(),
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    assert!(revoked.change_version > committed_version);
    let expected_url = "https://app.example.com/invites/secret".to_string();
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let response = crate::use_cases::repository_collaboration::publish_committed_mutation(
        &state,
        crate::use_cases::repository_collaboration::map_committed_mutation(invite, |invite| {
            (invite.0.id, expected_url.clone())
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
