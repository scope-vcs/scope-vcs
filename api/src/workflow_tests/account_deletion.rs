use super::*;
use crate::{
    clerk_users::{ClerkUserDeletion, ClerkUsers, ScriptedClerkUsers},
    use_cases::clerk_user_deletion::delete_due_clerk_users,
};

async fn request(app: &axum::Router, method: &str, path: &str, bearer: &str) -> Response {
    api_request(app.clone(), method, path, Some(bearer), None).await
}

async fn cli_bearer(app: &axum::Router) -> String {
    let grant = expect_json(
        request(app, "POST", "/v1/cli/exchange-grants", &bearer_header()).await,
        StatusCode::OK,
    )
    .await;
    let exchanged = expect_json(
        api_request(
            app.clone(),
            "POST",
            "/v1/cli/exchange-grants/exchange",
            None,
            Some(&serde_json::json!({ "exchange_token": grant["exchange_token"] }).to_string()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    format!("Bearer {}", exchanged["session_token"].as_str().unwrap())
}

fn scripted_clerk(state: &AppState) -> Arc<ScriptedClerkUsers> {
    match &state.clerk_users {
        ClerkUsers::Scripted(clerk) => clerk.clone(),
        _ => unreachable!("test state scripts Clerk"),
    }
}

#[tokio::test]
async fn deleting_the_account_ends_its_sessions_and_then_its_clerk_user() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let app = router(state.clone());
    let cli = cli_bearer(&app).await;

    let from_cli = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/account")
                .header(AUTHORIZATION, &cli)
                .header(
                    scope_api_contract::CLI_PROTOCOL_HEADER,
                    scope_api_contract::CLI_PROTOCOL_VERSION,
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(from_cli.status(), StatusCode::UNAUTHORIZED);

    let deleted = request(&app, "DELETE", "/v1/account", &bearer_header()).await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .is_err()
    );
    for bearer in [&cli, &bearer_header()] {
        let session = request(&app, "GET", "/v1/session", bearer).await;
        assert_eq!(session.status(), StatusCode::UNAUTHORIZED);
    }

    // Clerk is down for the first attempt; the Scope deletion stands and
    // the Clerk step waits for its retry.
    let clerk = scripted_clerk(&state);
    clerk
        .scripted
        .lock()
        .unwrap()
        .push_back(ClerkUserDeletion::Retry("Clerk answered 503".into()));
    let now = unix_now();
    for (at, claimed) in [(now, 1), (now + 29, 0), (now + 31, 1), (now + 3600, 0)] {
        assert_eq!(
            delete_due_clerk_users(&state, &move || Ok(at))
                .await
                .unwrap(),
            claimed,
            "at +{}",
            at - now
        );
    }
    assert_eq!(
        *clerk.attempts.lock().unwrap(),
        [TEST_CLERK_USER_ID, TEST_CLERK_USER_ID]
    );
    // A token issued before the deletion outlives it and must not recreate
    // the account once Clerk has confirmed.
    let session = request(&app, "GET", "/v1/session", &bearer_header()).await;
    assert_eq!(session.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_repository_other_members_use_blocks_deletion() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let member = test_user("user_member", "member", "member@example.com");
    state
        .metadata
        .auth()
        .insert_user_for_tests(member.clone())
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.members.push(RepositoryMember {
                repo_id: TEST_REPO_ID.into(),
                user_id: member.id.clone(),
                permissions: RepositoryMemberPermissions::default(),
                created_at_unix: 1,
                updated_at_unix: 1,
            });
        })
        .await
        .unwrap();

    let refused = expect_json(
        request(
            &router(state.clone()),
            "DELETE",
            "/v1/account",
            &bearer_header(),
        )
        .await,
        StatusCode::CONFLICT,
    )
    .await;

    assert_eq!(refused["code"], "shared_repositories");
    assert_eq!(
        refused["fields"]["repositories"],
        serde_json::json!([TEST_REPO_ID])
    );
    assert!(
        find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .is_ok()
    );
}
