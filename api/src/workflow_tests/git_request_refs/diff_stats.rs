use super::*;
use crate::use_cases::request_diff_stats::{DiffStatBackfillCursor, backfill_request_diff_stats};

fn diff_stat(additions: u64, deletions: u64, changed_files: u64) -> serde_json::Value {
    serde_json::json!({
        "additions": additions,
        "deletions": deletions,
        "changed_files": changed_files,
    })
}

async fn get_json(app: &axum::Router, uri: &str, bearer: &str) -> serde_json::Value {
    expect_json(
        api_request(app.clone(), "GET", uri, Some(bearer), None).await,
        StatusCode::OK,
    )
    .await
}

async fn listed_diff_stat(app: &axum::Router, bearer: &str) -> serde_json::Value {
    let list = get_json(app, &format!("/v1/repos/{TEST_REPO_ID}/requests"), bearer).await;
    list["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|request| request["id"] == REQUEST_ID)
        .unwrap()["diff_stat"]
        .clone()
}

async fn detail_diff_stat(app: &axum::Router, bearer: &str) -> serde_json::Value {
    get_json(
        app,
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}"),
        bearer,
    )
    .await["diff_stat"]
        .clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pushed_line_counts_reach_every_request_read_and_exclude_unreadable_files() {
    let state = test_state_with_request().await;
    let (source, remote, _server, _) = request_checkout(&state, "request-diff-stats").await;
    fs::write(source.join("logo.bin"), b"\x89PNG\0\x01binary").unwrap();
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "notes.txt",
        "one\ntwo\nthree\n",
        "add notes and logo",
    )
    .unwrap();
    state
        .metadata
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            event_id: "event_submit_for_diff_stats".to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();
    let app = router(state.clone());
    let owner = bearer_header();
    let public = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let full = diff_stat(4, 0, 3);

    let changes = get_json(
        &app,
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/changes"),
        &owner,
    )
    .await;
    let lines = changes["comparison"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| (file["path"].as_str().unwrap(), file["lines"].clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        lines,
        [
            ("logo.bin", serde_json::json!({ "kind": "binary" })),
            (
                "notes.txt",
                serde_json::json!({ "kind": "text", "additions": 3, "deletions": 0 })
            ),
            (
                "request.txt",
                serde_json::json!({ "kind": "text", "additions": 1, "deletions": 0 })
            ),
        ]
    );
    assert_eq!(listed_diff_stat(&app, &owner).await, full);
    assert_eq!(detail_diff_stat(&app, &owner).await, full);
    let queue = get_json(
        &app,
        &format!("/v1/repos/{TEST_REPO_ID}/requests/queue?section=unclaimed"),
        &owner,
    )
    .await;
    let queued = queue["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["request"]["id"] == REQUEST_ID)
        .unwrap();
    assert_eq!(queued["request"]["diff_stat"], full);

    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.policy
                .add_rule(LabelRule::private(ScopePath::parse("/notes.txt").unwrap()))
                .unwrap();
            repo.record.change_version += 1;
            repo.record.content_version = repo.record.change_version;
        })
        .await
        .unwrap();
    let restricted = diff_stat(1, 0, 2);
    assert_eq!(listed_diff_stat(&app, &public).await, restricted);
    assert_eq!(detail_diff_stat(&app, &public).await, restricted);
    assert_eq!(listed_diff_stat(&app, &owner).await, full);

    state
        .metadata
        .admin()
        .execute_for_tests("DELETE FROM scope_request_revision_diff_stats")
        .await
        .unwrap();
    assert_eq!(
        detail_diff_stat(&app, &owner).await,
        serde_json::Value::Null
    );
    let mut cursor = DiffStatBackfillCursor::default();
    assert_eq!(
        backfill_request_diff_stats(&state, &mut cursor)
            .await
            .unwrap(),
        1
    );
    assert_eq!(detail_diff_stat(&app, &owner).await, full);
    assert_eq!(detail_diff_stat(&app, &public).await, restricted);
}
