use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) type WriteBodies = BTreeMap<(&'static str, &'static str), String>;

pub(super) fn write_bodies(params: &BTreeMap<&'static str, String>) -> WriteBodies {
    let request = "/v1/repos/{owner}/{repo}/requests/{request_id}";
    let attachment = "/v1/repos/{owner}/{repo}/requests/{request_id}/attachments/{attachment_id}";
    let attempt = "/v1/runtime-protocol/attempts/{attempt_id}";
    let permissions =
        json!({ "can_push": false, "can_change_file_visibility": false, "view": "private" });
    let cache = json!({
        "cache_name": "isolation",
        "compatibility_inputs_digest": "isolation",
        "exact_inputs_digest": "isolation",
    });
    let reply = json!({
        "body_markdown": "isolation reply",
        "client_reply_id": "isolation-reply",
        "reply_to_reply_id": null,
        "wait_after_reply": false,
    });
    let succeeded = json!({ "conclusion": { "kind": "succeeded" }, "logs_truncated": false });
    let bodies: Vec<(&str, String, Value)> = vec![
        (
            "POST",
            "/v1/repos".into(),
            json!({ "name": "isolation-created" }),
        ),
        (
            "POST",
            "/v1/cli/browser-login".into(),
            json!({ "callback_url": "http://127.0.0.1:9/scope-cli-callback" }),
        ),
        (
            "POST",
            "/v1/cli/browser-login/{request_id}/exchange".into(),
            json!({ "request_secret": "unheld", "callback_code": "unheld" }),
        ),
        (
            "POST",
            "/v1/cli/exchange-grants/exchange".into(),
            json!({ "exchange_token": "unheld" }),
        ),
        (
            "POST",
            "/v1/github/setup".into(),
            json!({ "state": "unheld", "code": "unheld" }),
        ),
        (
            "POST",
            "/internal/cloud-dispatch/authorize".into(),
            json!({ "action": "stop", "attempt_id": params["attempt_id"] }),
        ),
        (
            "POST",
            "/v1/repos/{owner}/{repo}/github".into(),
            json!({ "grant": "unheld", "github_repository_id": 1, "acknowledge_public": false, "run_import_count": 0 }),
        ),
        (
            "PUT",
            "/v1/repos/{owner}/{repo}/github/main-publication".into(),
            json!({ "enabled": true }),
        ),
        (
            "PUT",
            "/v1/repos/{owner}/{repo}/github/required-checks".into(),
            json!({ "names": ["isolation"] }),
        ),
        (
            "PUT",
            "/v1/repos/{owner}/{repo}/github/review-bot-pull-requests".into(),
            json!({ "enabled": true }),
        ),
        (
            "PUT",
            "/v1/repos/{owner}/{repo}/github/review-bots".into(),
            json!({ "bots": [{ "login": "isolation-bot", "required": false }] }),
        ),
        (
            "PUT",
            "/v1/repos/{owner}/{repo}/github/review-request-comment".into(),
            json!({ "comment": "isolation" }),
        ),
        (
            "PUT",
            "/v1/repos/{owner}/{repo}/github/run-import".into(),
            json!({ "count": 1 }),
        ),
        (
            "POST",
            "/v1/repos/{owner}/{repo}/invites".into(),
            json!({ "email": "isolation-invite@example.com", "permissions": permissions }),
        ),
        (
            "PATCH",
            "/v1/repos/{owner}/{repo}/members/{member_user_id}".into(),
            json!({ "permissions": permissions }),
        ),
        (
            "POST",
            "/v1/repos/{owner}/{repo}/push-intents".into(),
            json!({ "head_oid": params["head_oid"], "view": "private" }),
        ),
        (
            "POST",
            "/v1/repos/{owner}/{repo}/requests".into(),
            json!({ "name": "isolation-started", "view": "private" }),
        ),
        (
            "PUT",
            format!("{request}/attention"),
            json!({ "action": "claim", "expected_activity_version": 1 }),
        ),
        (
            "PUT",
            format!("{request}/invitees"),
            json!({ "handle": "isolation-outsider" }),
        ),
        (
            "DELETE",
            format!("{request}/invitees"),
            json!({ "handle": "isolation-outsider" }),
        ),
        (
            "DELETE",
            format!("{request}/auto-merge"),
            json!({ "expected_intent_id": "isolation" }),
        ),
        (
            "POST",
            format!("{request}/auto-merge"),
            json!({ "expected_revision_id": params["revision_id"], "expected_head_oid": params["commit_oid"] }),
        ),
        (
            "POST",
            format!("{request}/checks/approve"),
            json!({ "expected_head_oid": params["commit_oid"] }),
        ),
        (
            "POST",
            format!("{request}/merge"),
            json!({ "expected_head_oid": params["commit_oid"] }),
        ),
        (
            "POST",
            format!("{request}/ratings"),
            json!({ "score": 1, "reason": "isolation" }),
        ),
        (
            "POST",
            format!("{request}/revision-context"),
            json!({
                "operation_key": "isolation",
                "base": { "kind": "main", "review_base_oid": params["head_oid"] },
                "expected_head_oid": params["commit_oid"],
                "expected_revision_id": params["revision_id"],
                "expected_base_version": 1,
                "expected_parent_base_version": null,
            }),
        ),
        (
            "POST",
            format!("{request}/timeline"),
            json!({ "body_markdown": "isolation", "client_discussion_id": "isolation", "anchor": null }),
        ),
        (
            "POST",
            format!("{request}/threads/{{discussion_id}}/replies"),
            reply.clone(),
        ),
        (
            "POST",
            format!("{request}/threads/{{discussion_id}}/reopen-and-reply"),
            reply,
        ),
        (
            "PUT",
            format!("{request}/threads/{{discussion_id}}/read"),
            json!({ "through_position": 1 }),
        ),
        (
            "POST",
            format!("{request}/attachments/prepare"),
            json!({
                "operation_id": "isolation",
                "target": { "kind": "Description", "discussion_id": null },
                "filename": "isolation.png",
                "declared_media_type": "image/png",
                "size_bytes": 1,
                "sha256": "0".repeat(64),
            }),
        ),
        (
            "POST",
            format!("{attachment}/finish"),
            json!({ "upload_id": params["upload_id"], "parts": [] }),
        ),
        (
            "POST",
            format!("{attachment}/media-grant"),
            json!({ "target": { "kind": "original" } }),
        ),
        (
            "POST",
            format!("{attachment}/retry"),
            json!({ "operation_id": "isolation" }),
        ),
        (
            "POST",
            format!("{attempt}/cache-observations/finalizations"),
            json!({ "caches": [] }),
        ),
        (
            "POST",
            format!("{attempt}/cache-observations/preparations"),
            json!({ "authorization_ms": 0, "wall_ms": 0, "caches": [] }),
        ),
        (
            "POST",
            format!("{attempt}/caches/restore"),
            json!({ "cache": cache }),
        ),
        (
            "POST",
            format!("{attempt}/caches/uploads/prepare"),
            json!({ "cache": cache, "object_digest": "isolation", "size_bytes": 1 }),
        ),
        (
            "POST",
            format!("{attempt}/caches/uploads/commit"),
            json!({ "cache": cache, "lease_id": "isolation", "object_digest": "isolation", "size_bytes": 1 }),
        ),
        ("POST", format!("{attempt}/complete"), succeeded.clone()),
        (
            "POST",
            format!("{attempt}/steps/{{step_index}}/complete"),
            succeeded,
        ),
        (
            "POST",
            format!("{attempt}/logs"),
            json!({ "step_index": 0, "sequence": 0, "text": "isolation" }),
        ),
    ];
    bodies
        .into_iter()
        .map(|(method, template, body)| {
            let template = scope_api_contract::routes::ROUTE_TEMPLATES
                .iter()
                .find(|known| **known == template)
                .unwrap_or_else(|| panic!("{method} {template} is not a route template"));
            ((method, *template), body.to_string())
        })
        .collect()
}
