use super::*;

async fn start_login(app: &axum::Router) -> serde_json::Value {
    let response = api_request(app.clone(), "POST", "/v1/cli/device-login", None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

#[tokio::test]
async fn cli_device_login_exchanges_browser_auth_for_cli_token() {
    let state = test_state_with_jwks();
    let app = router(state.clone());

    let start = start_login(&app).await;
    let device_code = start["device_code"].as_str().unwrap();
    let user_code = start["user_code"].as_str().unwrap();
    assert!(device_code.starts_with("scope_device_"));
    assert_eq!(user_code.len(), 16);
    assert_eq!(
        start["verification_url"].as_str().unwrap(),
        format!("{LOCAL_APP_ORIGIN}/cli-login")
    );

    let pending = api_request(
        app.clone(),
        "POST",
        &format!("/v1/cli/device-login/{}/{}", device_code, "poll"),
        None,
        None,
    )
    .await;
    assert_eq!(pending.status(), StatusCode::OK);
    let pending = response_json(pending).await;
    assert_eq!(pending["status"], "Pending");
    assert!(pending["session_token"].is_null());

    let complete = api_request(
        app.clone(),
        "POST",
        &format!("/v1/cli/device-login/{}/{}", user_code, "complete"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(complete.status(), StatusCode::OK);

    let authorized = api_request(
        app.clone(),
        "POST",
        &format!("/v1/cli/device-login/{}/{}", device_code, "poll"),
        None,
        None,
    )
    .await;
    assert_eq!(authorized.status(), StatusCode::OK);
    let authorized = response_json(authorized).await;
    assert_eq!(authorized["status"], "Complete");
    assert_eq!(authorized["identity"]["user_id"], test_owner_id());
    let cli_token = authorized["session_token"].as_str().unwrap();
    assert!(cli_token.starts_with(CLI_SESSION_TOKEN_PREFIX));

    let consumed = api_request(
        app.clone(),
        "POST",
        &format!("/v1/cli/device-login/{}/{}", device_code, "poll"),
        None,
        None,
    )
    .await;
    assert_eq!(consumed.status(), StatusCode::CONFLICT);

    let session = api_request(
        app.clone(),
        "GET",
        "/v1/session",
        Some(&format!("Bearer {cli_token}")),
        None,
    )
    .await;
    assert_eq!(session.status(), StatusCode::OK);
}

#[tokio::test]
async fn cli_device_login_completion_requires_clerk_auth_and_is_single_use() {
    let mut state = test_state_with_jwks();
    let (analytics, recording) = scope_product_analytics::ProductAnalytics::recording();
    state.product_analytics = analytics;
    let app = router(state);
    let start = start_login(&app).await;
    let user_code = start["user_code"].as_str().unwrap().to_string();

    let unauthorized = api_request(
        app.clone(),
        "POST",
        &format!("/v1/cli/device-login/{user_code}/complete"),
        Some(&format!("Bearer {CLI_SESSION_TOKEN_PREFIX}nope")),
        None,
    )
    .await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let first = api_request(
        app.clone(),
        "POST",
        &format!("/v1/cli/device-login/{user_code}/complete"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = api_request(
        app.clone(),
        "POST",
        &format!("/v1/cli/device-login/{user_code}/complete"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    assert_eq!(
        recording.event_names(),
        ["account:user_create", "operation:failure"]
    );
    assert_eq!(
        recording.property(1, "operation"),
        Some(serde_json::Value::String("login".into()))
    );
    assert_eq!(
        recording.property(1, "reason"),
        Some(serde_json::Value::String("conflict".into()))
    );
}

#[tokio::test]
async fn cli_login_routes_throttle_a_flooding_address_and_spare_others() {
    let app = router(AppState::test_state());
    let exchange = |address: &'static str| {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/cli/exchange-grants/exchange")
            .header("x-real-ip", address)
            .body(Body::empty())
            .unwrap();
        app.clone().oneshot(request)
    };
    let mut admitted = 0;
    while exchange("203.0.113.7").await.unwrap().status() != StatusCode::TOO_MANY_REQUESTS {
        admitted += 1;
        assert!(admitted < 1_000, "the flooding address was never throttled");
    }
    assert!(admitted > 1);
    assert_ne!(
        exchange("203.0.113.8").await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}
