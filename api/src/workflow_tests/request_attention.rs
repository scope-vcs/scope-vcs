use super::*;
use scope_domain::requests::RecordRequestRevisionInput;

const REQUEST_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn maintainer_attention_actions_preserve_claims_and_reject_stale_versions() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    super::requests::create_owner_request(&state, "req_attention_actions", REQUEST_HEAD).await;
    open_request(&state, "req_attention_actions", 10).await;
    let app = router(state);
    let bearer = bearer_header();

    let initial = queue_item(&app, "active", "req_attention_actions", Some(&bearer)).await;
    assert_eq!(initial["attention"]["reason"], "authored");
    let version = activity_version(&initial);

    let stale = attention_action(
        &app,
        "req_attention_actions",
        Some(&bearer),
        serde_json::json!({
            "action": "claim",
            "expected_activity_version": version + 1,
        }),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);

    let claimed = attention_json(
        &app,
        "req_attention_actions",
        &bearer,
        serde_json::json!({ "action": "claim", "expected_activity_version": version }),
    )
    .await;
    assert_eq!(claimed["attention"]["state"], "active");
    assert_eq!(claimed["attention"]["reason"], "claimed");
    assert_eq!(claimed["claimer"]["id"], test_owner_id());

    let settled = attention_json(
        &app,
        "req_attention_actions",
        &bearer,
        serde_json::json!({ "action": "settle", "expected_activity_version": version }),
    )
    .await;
    assert_eq!(settled["attention"]["state"], "settled");
    assert_eq!(settled["claimer"]["id"], test_owner_id());
    let aside = queue_item(&app, "set_aside", "req_attention_actions", Some(&bearer)).await;
    assert_eq!(aside["attention"]["can_restore"], true);
    assert_eq!(aside["claimer"]["id"], test_owner_id());

    let restored = attention_json(
        &app,
        "req_attention_actions",
        &bearer,
        serde_json::json!({ "action": "restore", "expected_activity_version": version }),
    )
    .await;
    assert_eq!(restored["attention"]["reason"], "restored");

    let snoozed_until = unix_now() + 600;
    let snoozed = attention_json(
        &app,
        "req_attention_actions",
        &bearer,
        serde_json::json!({
            "action": "snooze",
            "expected_activity_version": version,
            "until_unix": snoozed_until,
        }),
    )
    .await;
    assert_eq!(snoozed["attention"]["state"], "snoozed");
    assert_eq!(snoozed["attention"]["snoozed_until_unix"], snoozed_until);
    let aside = queue_item(&app, "set_aside", "req_attention_actions", Some(&bearer)).await;
    assert_eq!(aside["claimer"]["id"], test_owner_id());
}

#[tokio::test]
async fn reply_wait_is_atomic_and_only_other_activity_reactivates_attention() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let author_id =
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", "attention_author");
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(
            &author_id,
            "attention-author",
            "attention-author@example.com",
        ))
        .await
        .unwrap();
    super::requests::create_public_request(
        &state,
        "req_attention_activity",
        author_id.clone(),
        REQUEST_HEAD,
    )
    .await;
    open_request(&state, "req_attention_activity", 10).await;
    let app = router(state.clone());
    let owner = bearer_header();
    let author = bearer_header_for("attention_author", "attention-author@example.com");
    let base = "/v1/repos/owner/repo/requests/req_attention_activity";

    let forbidden = attention_action(
        &app,
        "req_attention_activity",
        Some(&author),
        serde_json::json!({ "action": "claim", "expected_activity_version": 2 }),
    )
    .await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let discussion = api_request(
        app.clone(),
        "POST",
        &format!("{base}/timeline"),
        Some(&author),
        Some(r#"{"body_markdown":"Please review this.","client_discussion_id":"attention-root"}"#),
    )
    .await;
    assert_eq!(discussion.status(), StatusCode::OK);
    let discussion_id = response_json(discussion).await["discussion"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let initial = queue_item(&app, "unclaimed", "req_attention_activity", Some(&owner)).await;
    let version = activity_version(&initial);
    attention_json(
        &app,
        "req_attention_activity",
        &owner,
        serde_json::json!({ "action": "claim", "expected_activity_version": version }),
    )
    .await;

    let active = queue_item(&app, "active", "req_attention_activity", Some(&owner)).await;
    attention_json(
        &app,
        "req_attention_activity",
        &owner,
        serde_json::json!({
            "action": "settle",
            "expected_activity_version": activity_version(&active),
        }),
    )
    .await;
    let settled = queue_item(&app, "set_aside", "req_attention_activity", Some(&owner)).await;
    let settled_through = settled["attention"]["through_activity_version"]
        .as_u64()
        .unwrap();

    let edited = api_request(
        app.clone(),
        "PATCH",
        base,
        Some(&author),
        Some(r#"{"title":"Attention title edit"}"#),
    )
    .await;
    assert_eq!(edited.status(), StatusCode::OK);
    assert_eq!(
        queue_item(&app, "set_aside", "req_attention_activity", Some(&owner)).await["attention"]["reason"],
        "settled"
    );

    let ordinary = api_request(
        app.clone(),
        "POST",
        &format!("{base}/threads/{discussion_id}/replies"),
        Some(&owner),
        Some(
            r#"{"body_markdown":"Ordinary owner reply.","client_reply_id":"ordinary-owner","reply_to_reply_id":null,"wait_after_reply":false}"#,
        ),
    )
    .await;
    assert_eq!(ordinary.status(), StatusCode::OK);
    let ordinary = response_json(ordinary).await;
    let ordinary_position = ordinary["reply"]["position"].as_u64().unwrap();
    let read = api_request(
        app.clone(),
        "PUT",
        &format!("{base}/threads/{discussion_id}/read"),
        Some(&owner),
        Some(&format!(r#"{{"through_position":{ordinary_position}}}"#)),
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    let still_settled = queue_item(&app, "set_aside", "req_attention_activity", Some(&owner)).await;
    assert_eq!(still_settled["attention"]["reason"], "settled");
    assert_eq!(
        still_settled["attention"]["through_activity_version"],
        settled_through
    );

    let wait_reply = api_request(
        app.clone(),
        "POST",
        &format!("{base}/threads/{discussion_id}/replies"),
        Some(&owner),
        Some(
            r#"{"body_markdown":"Waiting after this reply.","client_reply_id":"wait-owner","reply_to_reply_id":null,"wait_after_reply":true}"#,
        ),
    )
    .await;
    assert_eq!(wait_reply.status(), StatusCode::OK);
    let wait_reply = response_json(wait_reply).await;
    let wait_position = wait_reply["reply"]["position"].as_u64().unwrap();
    let waiting = queue_item(&app, "set_aside", "req_attention_activity", Some(&owner)).await;
    assert_eq!(waiting["attention"]["state"], "waiting");
    assert_eq!(
        waiting["attention"]["through_activity_version"],
        wait_position
    );

    post_reply(&app, base, &discussion_id, &author, "incoming-after-wait").await;
    let woke_from_wait = queue_item(&app, "active", "req_attention_activity", Some(&owner)).await;
    assert_eq!(woke_from_wait["attention"]["reason"], "new_activity");

    attention_json(
        &app,
        "req_attention_activity",
        &owner,
        serde_json::json!({
            "action": "settle",
            "expected_activity_version": activity_version(&woke_from_wait),
        }),
    )
    .await;
    let mut snapshot = source_blob(&state, "incoming request revision");
    let revision_head = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    snapshot.git_oid = revision_head.to_string();
    state
        .metadata
        .requests()
        .record_request_revision(
            RecordRequestRevisionInput {
                request_id: "req_attention_activity".to_string(),
                actor_user_id: author_id,
                actor_can_edit: true,
                expected_old_head_oid: Some(REQUEST_HEAD.to_string()),
                new_head_oid: revision_head.to_string(),
                git_snapshot: snapshot,
                event_id: "attention-incoming-revision".to_string(),
                body: None,
                now_unix: 50,
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    let woke_from_revision =
        queue_item(&app, "active", "req_attention_activity", Some(&owner)).await;
    assert_eq!(woke_from_revision["attention"]["reason"], "new_activity");

    attention_json(
        &app,
        "req_attention_activity",
        &owner,
        serde_json::json!({
            "action": "snooze",
            "expected_activity_version": activity_version(&woke_from_revision),
            "until_unix": unix_now() + 600,
        }),
    )
    .await;
    post_reply(&app, base, &discussion_id, &author, "incoming-after-snooze").await;
    let woke_from_snooze = queue_item(&app, "active", "req_attention_activity", Some(&owner)).await;
    assert_eq!(woke_from_snooze["attention"]["reason"], "new_activity");
}

#[tokio::test]
async fn public_queue_exposes_open_and_terminal_requests_in_nested_rows() {
    let state = test_state_with_readme().await;
    super::requests::create_public_request(
        &state,
        "req_public_active",
        test_owner_id(),
        REQUEST_HEAD,
    )
    .await;
    super::requests::create_public_request(
        &state,
        "req_public_closed",
        test_owner_id(),
        REQUEST_HEAD,
    )
    .await;
    open_request(&state, "req_public_active", 10).await;
    close_request(&state, "req_public_closed", 11, 12).await;
    let app = router(state);

    let active = queue_item(&app, "active", "req_public_active", None).await;
    assert_eq!(active["request"]["state"], "Open");
    assert_eq!(active["author"]["id"], test_owner_id());
    assert_eq!(active["attention"]["reason"], "open");
    assert!(active["claimer"].is_null());

    let closed = queue_item(&app, "set_aside", "req_public_closed", None).await;
    assert_eq!(closed["request"]["state"], "Closed");
    assert_eq!(closed["attention"]["reason"], "closed");
    let unclaimed = queue_page(&app, "unclaimed", None).await;
    assert!(unclaimed["requests"].as_array().unwrap().is_empty());
}

async fn open_request(state: &AppState, request_id: &str, now_unix: u64) {
    state
        .metadata
        .requests()
        .mutate_request_for_tests(request_id, |request| {
            request.submitted_at_unix = Some(now_unix);
            request.updated_at_unix = now_unix;
        })
        .await
        .unwrap();
}

async fn close_request(
    state: &AppState,
    request_id: &str,
    submitted_at_unix: u64,
    closed_at_unix: u64,
) {
    state
        .metadata
        .requests()
        .mutate_request_for_tests(request_id, |request| {
            request.submitted_at_unix = Some(submitted_at_unix);
            request.closed_at_unix = Some(closed_at_unix);
            request.closed_by_user_id = Some(test_owner_id());
            request.updated_at_unix = closed_at_unix;
        })
        .await
        .unwrap();
}

async fn queue_page(app: &axum::Router, section: &str, bearer: Option<&str>) -> serde_json::Value {
    let response = api_request(
        app.clone(),
        "GET",
        &format!("/v1/repos/owner/repo/requests/queue?section={section}"),
        bearer,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

async fn queue_item(
    app: &axum::Router,
    section: &str,
    request_id: &str,
    bearer: Option<&str>,
) -> serde_json::Value {
    queue_page(app, section, bearer).await["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["request"]["id"] == request_id)
        .unwrap_or_else(|| panic!("request {request_id} missing from {section}"))
        .clone()
}

fn activity_version(item: &serde_json::Value) -> u64 {
    item["attention"]["activity_version"].as_u64().unwrap()
}

async fn attention_action(
    app: &axum::Router,
    request_id: &str,
    bearer: Option<&str>,
    body: serde_json::Value,
) -> Response {
    api_request(
        app.clone(),
        "PUT",
        &format!("/v1/repos/owner/repo/requests/{request_id}/attention"),
        bearer,
        Some(&body.to_string()),
    )
    .await
}

async fn attention_json(
    app: &axum::Router,
    request_id: &str,
    bearer: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let response = attention_action(app, request_id, Some(bearer), body).await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

async fn post_reply(
    app: &axum::Router,
    base: &str,
    discussion_id: &str,
    bearer: &str,
    client_reply_id: &str,
) {
    let response = api_request(
        app.clone(),
        "POST",
        &format!("{base}/threads/{discussion_id}/replies"),
        Some(bearer),
        Some(
            &serde_json::json!({
                "body_markdown": client_reply_id,
                "client_reply_id": client_reply_id,
                "reply_to_reply_id": null,
                "wait_after_reply": false,
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}
