use super::*;

const INVITED_EMAIL: &str = "invitee@example.com";

fn invitee_header() -> String {
    bearer_header_for("user_invitee", INVITED_EMAIL)
}

async fn json_request(
    state: &AppState,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let body = body.map(|body| body.to_string());
    let response = api_request(router(state.clone()), method, path, bearer, body.as_deref()).await;
    let status = response.status();
    (status, response_json(response).await)
}

/// Creates an invite through the API and returns its id and first link token.
async fn create_invite(state: &AppState) -> (String, String) {
    let (status, body) = json_request(
        state,
        "POST",
        "/v1/repos/owner/repo/invites",
        Some(&bearer_header()),
        Some(serde_json::json!({
            "email": INVITED_EMAIL,
            "permissions": RepositoryMemberPermissions::default(),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (
        body["invite"]["id"].as_str().unwrap().to_string(),
        link_token(&body),
    )
}

fn link_token(body: &serde_json::Value) -> String {
    let invite_url = body["invite_url"].as_str().unwrap();
    invite_url.rsplit('/').next().unwrap().to_string()
}

async fn landing(state: &AppState, token: &str, bearer: Option<&str>) -> serde_json::Value {
    let (status, body) = json_request(
        state,
        "GET",
        &format!("/v1/repository-invites/{token}"),
        bearer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body
}

async fn accept(state: &AppState, token: &str, bearer: &str) -> (StatusCode, serde_json::Value) {
    json_request(
        state,
        "POST",
        &format!("/v1/repository-invites/{token}/accept"),
        Some(bearer),
        None,
    )
    .await
}

#[tokio::test]
async fn acceptance_grants_member_access_and_can_be_repeated_safely() {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    let (analytics, recording) = scope_product_analytics::ProductAnalytics::recording();
    state.product_analytics = analytics;
    let (_, token) = create_invite(&state).await;

    let signed_out = landing(&state, &token, None).await;
    assert_eq!(signed_out["status"], "open");
    assert_eq!(signed_out["viewer"], "signed_out");
    assert_eq!(signed_out["invited_email"], INVITED_EMAIL);
    let stranger = bearer_header_for("user_stranger", "stranger@example.com");
    let wrong_account = landing(&state, &token, Some(&stranger)).await;
    assert_eq!(wrong_account["viewer"], "wrong_account");
    assert_eq!(wrong_account["viewer_email"], "stranger@example.com");
    assert_eq!(
        accept(&state, &token, &stranger).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        landing(&state, &token, Some(&invitee_header())).await["viewer"],
        "ready"
    );

    let (status, body) = accept(&state, &token, &invitee_header()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["repo"]["access"]["actor"], "Member");
    let accepted_events = [
        "account:user_create",
        "account:user_create",
        "repository:invite_accept",
    ];
    assert_eq!(recording.event_names(), accepted_events);
    assert_eq!(
        recording.property(2, "repository_id"),
        Some(serde_json::Value::String("repoi_workflow_test".into()))
    );
    assert_eq!(
        recording.property(2, "actor_role"),
        Some(serde_json::Value::String("member".into()))
    );

    // A double click or a retry after a lost response succeeds again without
    // adding a second membership or a second analytics event.
    let (status, repeated) = accept(&state, &token, &invitee_header()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repeated["member"], body["member"]);
    assert_eq!(recording.event_names(), accepted_events);

    assert_eq!(
        landing(&state, &token, Some(&invitee_header())).await["status"],
        "member"
    );
    // A used link tells anyone else only that it was used.
    assert_eq!(
        landing(&state, &token, Some(&stranger)).await,
        serde_json::json!({ "status": "used" })
    );
    assert_eq!(
        landing(&state, &token, None).await,
        serde_json::json!({ "status": "used" })
    );
}

#[tokio::test]
async fn every_issued_link_works_until_the_invite_is_revoked() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let (invite_id, first) = create_invite(&state).await;
    let links_path = format!("/v1/repos/owner/repo/invites/{invite_id}/links");

    let (status, body) =
        json_request(&state, "POST", &links_path, Some(&bearer_header()), None).await;
    assert_eq!(status, StatusCode::OK);
    let second = link_token(&body);
    assert_ne!(first, second);
    let first_landing = landing(&state, &first, None).await;
    assert_eq!(first_landing["status"], "open");
    // A new link does not extend the invite.
    assert_eq!(landing(&state, &second, None).await, first_landing);

    // Only the owner can issue links. The repository is private, so anyone
    // else is told it does not exist.
    let (status, _) =
        json_request(&state, "POST", &links_path, Some(&invitee_header()), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A second invite for the same email is refused while one is pending.
    let (status, _) = json_request(
        &state,
        "POST",
        "/v1/repos/owner/repo/invites",
        Some(&bearer_header()),
        Some(serde_json::json!({
            "email": INVITED_EMAIL,
            "permissions": RepositoryMemberPermissions::default(),
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, revoked) = json_request(
        &state,
        "DELETE",
        &format!("/v1/repos/owner/repo/invites/{invite_id}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revoked["state"], "Revoked");

    for token in [&first, &second] {
        assert_eq!(
            landing(&state, token, Some(&invitee_header())).await,
            serde_json::json!({ "status": "revoked" })
        );
        assert_eq!(
            accept(&state, token, &invitee_header()).await.0,
            StatusCode::CONFLICT
        );
    }
    let (status, _) = json_request(&state, "POST", &links_path, Some(&bearer_header()), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn an_expired_invite_reads_as_expired_everywhere_and_can_be_replaced() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let token = "expired-invite-token";
    let now = unix_now();
    let invite = RepositoryInvite {
        id: "invite_expired".into(),
        repo_id: TEST_REPO_ID.into(),
        invited_email: INVITED_EMAIL.into(),
        invited_email_normalized: INVITED_EMAIL.into(),
        permissions: RepositoryMemberPermissions::default(),
        invited_by_user_id: test_owner_id(),
        link_hashes: vec![token_hash(token)],
        created_at_unix: now - 700,
        updated_at_unix: now - 700,
        expires_at_unix: now - 100,
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

    let expired = landing(&state, token, Some(&invitee_header())).await;
    assert_eq!(expired["status"], "expired");
    assert_eq!(expired["repo_name"], "repo");
    assert!(expired.get("invited_email").is_none());
    assert_eq!(
        accept(&state, token, &invitee_header()).await.0,
        StatusCode::CONFLICT
    );
    let (_, members) = json_request(
        &state,
        "GET",
        "/v1/repos/owner/repo/members",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(members["invites"][0]["state"], "Expired");

    // The expired invite does not block a new one, and its link stays dead.
    let (_, fresh) = create_invite(&state).await;
    assert_eq!(landing(&state, &fresh, None).await["status"], "open");
    assert_eq!(landing(&state, token, None).await["status"], "expired");
}

#[tokio::test]
async fn a_removed_member_cannot_rejoin_by_replaying_the_link() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let (_, token) = create_invite(&state).await;
    let (status, body) = accept(&state, &token, &invitee_header()).await;
    assert_eq!(status, StatusCode::OK);
    let member_user_id = body["member"]["user_id"].as_str().unwrap();

    let (status, _) = json_request(
        &state,
        "DELETE",
        &format!("/v1/repos/owner/repo/members/{member_user_id}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(
        landing(&state, &token, Some(&invitee_header())).await,
        serde_json::json!({ "status": "access_removed" })
    );
    assert_eq!(
        accept(&state, &token, &invitee_header()).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn an_unknown_link_lands_as_invalid() {
    let state = test_state_with_repo();
    assert_eq!(
        landing(&state, "scope_invite_unknown", None).await,
        serde_json::json!({ "status": "invalid" })
    );
}
