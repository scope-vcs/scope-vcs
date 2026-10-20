use super::super::super::fake_github::webhook;
use super::*;
use crate::use_cases::github_review_imports::import_due_github_reviews;
use scope_api_contract::routes::repo_request_action;

const BOT_ID: u64 = 136622811;
const REVIEW_ID: u64 = 101;

fn bot() -> serde_json::Value {
    serde_json::json!({
        "id": BOT_ID,
        "login": "greptile-apps[bot]",
        "type": "Bot",
        "avatar_url": "https://avatars.githubusercontent.com/in/867647",
    })
}

fn line_comment(
    id: u64,
    head: &str,
    path: &str,
    start_line: Option<u32>,
    line: u32,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "pull_request_review_id": REVIEW_ID,
        "in_reply_to_id": null,
        "user": bot(),
        "body": format!("Comment {id}"),
        "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1#discussion_r{id}"),
        "path": path,
        "subject_type": "line",
        "side": "RIGHT",
        "start_line": start_line,
        "line": line,
        "original_start_line": start_line,
        "original_line": line,
        "commit_id": head,
        "original_commit_id": head,
        "created_at": "2026-10-20T12:00:00Z",
    })
}

async fn deliver_review(owner: &OwnerRequest, head_repository_id: u64) {
    let delivery = webhook(
        &owner.state,
        "pull_request_review",
        serde_json::json!({
            "action": "submitted",
            "repository": { "id": GITHUB_REPOSITORY_ID },
            "pull_request": {
                "number": 1,
                "head": { "ref": owner.branch(), "repo": { "id": head_repository_id } },
            },
            "review": { "id": REVIEW_ID },
            "sender": bot(),
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
}

async fn threads(owner: &OwnerRequest) -> Vec<serde_json::Value> {
    let page = expect_json(
        api_request(
            router(owner.state.clone()),
            "GET",
            &repo_request_action(
                TEST_REPO_OWNER,
                TEST_REPO_NAME,
                &owner.request_id,
                "timeline",
            ),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    page["discussions"].as_array().unwrap().clone()
}

fn by_client_id<'a>(threads: &'a [serde_json::Value], client_id: &str) -> &'a serde_json::Value {
    threads
        .iter()
        .find(|thread| thread["client_discussion_id"] == client_id)
        .unwrap_or_else(|| panic!("missing {client_id}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bot_review_on_the_carrier_pull_request_becomes_anchored_discussions() {
    let owner = owner_request("review-import", &[]).await;
    let head = owner.head();
    owner.fake.report_pull_review(
        serde_json::json!({
            "id": REVIEW_ID,
            "user": bot(),
            "body": "Two things to look at.",
            "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1#pullrequestreview-{REVIEW_ID}"),
            "commit_id": head,
            "submitted_at": "2026-10-20T12:00:00Z",
        }),
        vec![
            line_comment(201, &head, "request.txt", None, 1),
            line_comment(202, &head, "request.txt", Some(1), 40),
            line_comment(203, &head, "missing.txt", None, 1),
        ],
    );

    deliver_review(&owner, GITHUB_REPOSITORY_ID + 1).await;
    assert_eq!(
        import_due_github_reviews(&owner.state, unix_now())
            .await
            .unwrap(),
        0
    );
    assert!(threads(&owner).await.is_empty());

    deliver_review(&owner, GITHUB_REPOSITORY_ID).await;
    assert_eq!(
        import_due_github_reviews(&owner.state, unix_now())
            .await
            .unwrap(),
        1
    );
    deliver_review(&owner, GITHUB_REPOSITORY_ID).await;
    assert_eq!(
        import_due_github_reviews(&owner.state, unix_now())
            .await
            .unwrap(),
        1
    );

    let threads = threads(&owner).await;
    assert_eq!(threads.len(), 4);
    let line = by_client_id(&threads, "github:review-comment:201");
    assert_eq!(line["author"]["kind"], "github");
    assert_eq!(line["author"]["account"]["login"], "greptile-apps[bot]");
    assert_eq!(line["author"]["account"]["github_user_id"], BOT_ID);
    assert_eq!(line["author"]["account"]["account_type"], "bot");
    assert_eq!(
        line["author"]["source_url"],
        format!("https://github.com/{GITHUB_FULL_NAME}/pull/1#discussion_r201")
    );
    assert_eq!(line["anchor"]["path"], "/request.txt");
    assert_eq!(
        line["anchor"]["lines"],
        serde_json::json!({ "side": "new", "start_line": 1, "end_line": 1 })
    );
    let past_the_end = by_client_id(&threads, "github:review-comment:202");
    assert_eq!(past_the_end["anchor"]["path"], "/request.txt");
    assert!(past_the_end["anchor"]["lines"].is_null());
    let unchanged_path = by_client_id(&threads, "github:review-comment:203");
    assert!(unchanged_path["anchor"]["path"].is_null());
    assert_eq!(
        unchanged_path["anchor"]["revision_id"],
        line["anchor"]["revision_id"]
    );
    let body = by_client_id(&threads, "github:review:101");
    assert_eq!(body["body_markdown"], "Two things to look at.");
    assert_eq!(body["anchor"]["revision_id"], line["anchor"]["revision_id"]);
    assert!(body["anchor"]["path"].is_null());
}
