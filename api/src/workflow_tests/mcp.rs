use super::*;
use crate::{
    clerk_backend::{ClerkOAuthAccessToken, ClerkOAuthTokenCheck},
    http::origins::public_mcp_resource,
};
use scope_api_contract::routes;
use serde_json::{Value, json};

const OWNER_AGENT: &str = "Bearer oat_owner";
const OUTSIDER_AGENT: &str = "Bearer oat_outsider";
const OUTSIDER_ID: &str = "user_outsider";
const OUTSIDER_EMAIL: &str = "outsider@example.com";

async fn signed_in_app() -> (AppState, axum::Router) {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let mut repo = repo_with_readme(&state);
    let path = ScopePath::parse("/secret.txt").unwrap();
    let content = source_blob(&state, "owner only");
    repo.graph.commits[0].changes.push(FileChange {
        label: ViewId::private(),
        path: path.clone(),
        old_content: None,
        new_content: Some(content.clone()),
    });
    repo.live_files.insert(path, content);
    replace_test_repo(&state, repo).await;
    drain_outbox(&state, "mcp").await;
    let app = router(state.clone());
    for session in [
        bearer_header(),
        bearer_header_for(OUTSIDER_ID, OUTSIDER_EMAIL),
    ] {
        let response = get(app.clone(), "/v1/repos/owner/repo", Some(&session)).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    (state, app)
}

fn grant(state: &AppState, bearer: &str, subject: &str, scopes: &[&str], audience: &str) {
    scripted_clerk(state).oauth_tokens.lock().unwrap().insert(
        bearer.trim_start_matches("Bearer ").to_string(),
        ClerkOAuthTokenCheck::Active(ClerkOAuthAccessToken {
            subject: subject.to_string(),
            scopes: scopes.iter().map(|scope| scope.to_string()).collect(),
            audiences: vec![audience.to_string()],
        }),
    );
}

async fn rpc(
    app: &axum::Router,
    bearer: Option<&str>,
    headers: &[(&str, &str)],
    body: Value,
) -> Response {
    let mut request = Request::builder()
        .method("POST")
        .uri(routes::MCP)
        .header(CONTENT_TYPE, "application/json");
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, bearer);
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

fn legacy_call(tool: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    })
}

fn modern(method: &str, version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "discover",
        "method": method,
        "params": { "_meta": {
            "io.modelcontextprotocol/protocolVersion": version,
            "io.modelcontextprotocol/clientCapabilities": {},
        } },
    })
}

fn challenge(response: &Response) -> String {
    response.headers()[WWW_AUTHENTICATE]
        .to_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn an_agent_without_a_token_is_sent_to_sign_in_through_the_resource_metadata() {
    let (_, app) = signed_in_app().await;

    let refused = rpc(&app, None, &[], legacy_call("repo_list", json!({}))).await;

    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let challenge = challenge(&refused);
    assert!(challenge.contains(r#"scope="read""#), "{challenge}");
    let metadata_url = challenge
        .split(r#"resource_metadata=""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap();
    let metadata_path = metadata_url
        .strip_prefix(crate::config::LOCAL_API_ORIGIN)
        .unwrap_or(metadata_url);
    let metadata = expect_json(get(app, metadata_path, None).await, StatusCode::OK).await;
    assert_eq!(metadata["resource"], public_mcp_resource().unwrap());
    assert_eq!(
        metadata["authorization_servers"],
        json!([TEST_CLERK_ISSUER])
    );
}

#[tokio::test]
async fn only_active_agent_tokens_issued_for_this_endpoint_reach_the_tools() {
    let (state, app) = signed_in_app().await;
    let resource = public_mcp_resource().unwrap();
    grant(
        &state,
        "Bearer oat_elsewhere",
        TEST_CLERK_USER_ID,
        &["read"],
        "https://elsewhere.example/mcp",
    );
    grant(
        &state,
        "Bearer oat_stranger",
        "user_never_signed_in",
        &["read"],
        &resource,
    );
    grant(
        &state,
        OWNER_AGENT,
        TEST_CLERK_USER_ID,
        &["read"],
        &resource,
    );

    for refused in [
        "Bearer oat_revoked",
        "Bearer oat_elsewhere",
        "Bearer oat_stranger",
        &bearer_header(),
    ] {
        let response = rpc(
            &app,
            Some(refused),
            &[],
            legacy_call("repo_list", json!({})),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{refused}");
    }
    let listed = expect_json(
        rpc(
            &app,
            Some(OWNER_AGENT),
            &[],
            legacy_call("repo_list", json!({})),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(listed["result"]["isError"], false);
    assert_eq!(
        listed["result"]["structuredContent"]["repositories"][0]["name"],
        "repo"
    );
}

#[tokio::test]
async fn a_tool_needs_its_permission_and_reads_only_what_the_signed_in_user_can() {
    let (state, app) = signed_in_app().await;
    let resource = public_mcp_resource().unwrap();
    grant(
        &state,
        "Bearer oat_profile",
        TEST_CLERK_USER_ID,
        &["profile", "email"],
        &resource,
    );
    grant(
        &state,
        OWNER_AGENT,
        TEST_CLERK_USER_ID,
        &["read"],
        &resource,
    );
    grant(&state, OUTSIDER_AGENT, OUTSIDER_ID, &["read"], &resource);
    let read_secret = legacy_call(
        "file_read",
        json!({ "repo": "owner/repo", "path": "secret.txt" }),
    );

    let unscoped = rpc(&app, Some("Bearer oat_profile"), &[], read_secret.clone()).await;
    assert_eq!(unscoped.status(), StatusCode::FORBIDDEN);
    let challenge = challenge(&unscoped);
    assert!(
        challenge.contains(r#"error="insufficient_scope""#),
        "{challenge}"
    );
    assert!(challenge.contains(r#"scope="read""#), "{challenge}");

    let owner = expect_json(
        rpc(&app, Some(OWNER_AGENT), &[], read_secret.clone()).await,
        StatusCode::OK,
    )
    .await;
    let http = expect_json(
        get(
            app.clone(),
            "/v1/repos/owner/repo/files/content?path=secret.txt",
            Some(&bearer_header()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(owner["result"]["structuredContent"], http);

    let outsider = expect_json(
        rpc(&app, Some(OUTSIDER_AGENT), &[], read_secret).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(outsider["result"]["isError"], true);
    assert!(
        outsider["result"]["structuredContent"]
            .get("content")
            .is_none()
    );
}

#[tokio::test]
async fn current_and_handshake_era_clients_are_both_answered() {
    let (state, app) = signed_in_app().await;
    grant(
        &state,
        OWNER_AGENT,
        TEST_CLERK_USER_ID,
        &["read"],
        &public_mcp_resource().unwrap(),
    );
    let agent = Some(OWNER_AGENT);
    let current = [
        ("mcp-protocol-version", "2026-07-28"),
        ("mcp-method", "server/discover"),
    ];

    for (requested, negotiated) in [("2025-06-18", "2025-06-18"), ("2024-11-05", "2025-11-25")] {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": { "protocolVersion": requested, "capabilities": {}, "clientInfo": { "name": "codex" } },
        });
        let answered = expect_json(rpc(&app, agent, &[], initialize).await, StatusCode::OK).await;
        assert_eq!(answered["result"]["protocolVersion"], negotiated);
    }
    let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert_eq!(
        rpc(&app, agent, &[], initialized).await.status(),
        StatusCode::ACCEPTED
    );

    let discovered = expect_json(
        rpc(
            &app,
            agent,
            &current,
            modern("server/discover", "2026-07-28"),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        discovered["result"]["supportedVersions"],
        json!(["2026-07-28"])
    );
    assert_eq!(discovered["result"]["resultType"], "complete");
    assert_eq!(discovered["result"]["cacheScope"], "public");
    assert!(discovered["result"]["ttlMs"].is_u64());

    let mismatched = [
        ("mcp-protocol-version", "2026-07-28"),
        ("mcp-method", "tools/list"),
    ];
    let mismatch = expect_json(
        rpc(
            &app,
            agent,
            &mismatched,
            modern("server/discover", "2026-07-28"),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(mismatch["error"]["code"], -32020);

    let unsupported = expect_json(
        rpc(
            &app,
            agent,
            &current,
            modern("server/discover", "2099-01-01"),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(unsupported["error"]["code"], -32022);
    assert_eq!(unsupported["error"]["data"]["supported"][0], "2026-07-28");

    let foreign_origin = [("origin", "https://attacker.example")];
    assert_eq!(
        rpc(
            &app,
            agent,
            &foreign_origin,
            legacy_call("repo_list", json!({}))
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
}
