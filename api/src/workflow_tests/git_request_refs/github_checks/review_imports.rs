use super::super::super::fake_github::webhook;
use super::*;
use crate::use_cases::github_review_imports::import_due_github_reviews;
use scope_api_contract::routes::repo_request_action;
use scope_domain::github_review_bots::GitHubReviewBotInput;

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

fn user(id: u64, login: &str, account_type: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "login": login, "type": account_type, "avatar_url": null })
}

fn review(id: u64, author: serde_json::Value, head: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "user": author,
        "body": format!("Review {id}"),
        "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1#pullrequestreview-{id}"),
        "commit_id": head,
        "submitted_at": "2026-10-20T12:00:00Z",
    })
}

fn authored(mut comment: serde_json::Value, author: &serde_json::Value) -> serde_json::Value {
    comment["user"] = author.clone();
    comment
}

async fn list_review_bots(owner: &OwnerRequest, logins: &[&str]) {
    owner
        .state
        .metadata
        .repositories()
        .set_github_review_bots(
            TEST_REPO_ID,
            &test_owner_id(),
            logins
                .iter()
                .map(|login| GitHubReviewBotInput {
                    login: login.to_string(),
                })
                .collect(),
        )
        .await
        .unwrap();
}

async fn import_pass(owner: &OwnerRequest) -> usize {
    import_due_github_reviews(&owner.state, unix_now())
        .await
        .unwrap()
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
    deliver_review_id(owner, REVIEW_ID, head_repository_id).await;
}

async fn deliver_review_id(owner: &OwnerRequest, review_id: u64, head_repository_id: u64) {
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
            "review": { "id": review_id },
            "sender": bot(),
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
}

async fn deliver_thread(
    owner: &OwnerRequest,
    action: &str,
    root_comment_id: u64,
    sender: serde_json::Value,
) {
    let delivery = webhook(
        &owner.state,
        "pull_request_review_thread",
        serde_json::json!({
            "action": action,
            "repository": { "id": GITHUB_REPOSITORY_ID },
            "pull_request": {
                "number": 1,
                "head": { "ref": owner.branch(), "repo": { "id": GITHUB_REPOSITORY_ID } },
            },
            "thread": {
                "node_id": format!("PRRT_{root_comment_id}"),
                "comments": [{ "id": root_comment_id, "in_reply_to_id": null }],
            },
            "sender": sender,
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
    list_review_bots(&owner, &["greptile-apps[bot]"]).await;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_listed_review_bots_and_github_users_who_can_write_are_imported() {
    let owner = owner_request("review-import-authors", &[]).await;
    list_review_bots(&owner, &["greptile-apps[bot]"]).await;
    let head = owner.head();
    let spam = user(501, "spam-reviewer[bot]", "Bot");
    let writer = user(502, "octo-writer", "User");
    let reader = user(503, "octo-reader", "User");
    let hidden = user(504, "octo-hidden", "User");
    let renamed = user(505, "octo-renamed", "User");
    owner
        .fake
        .report_permission("octo-writer", Some(("write", 502)));
    owner
        .fake
        .report_permission("octo-reader", Some(("read", 503)));
    owner.fake.report_permission("octo-hidden", None);
    owner
        .fake
        .report_permission("octo-renamed", Some(("admin", 999)));
    for (review_id, author, comment_ids) in [
        (301, &spam, vec![311]),
        (302, &writer, vec![321, 322]),
        (303, &reader, vec![331]),
        (304, &hidden, vec![341]),
        (305, &renamed, vec![351]),
    ] {
        owner.fake.report_pull_review(
            review(review_id, author.clone(), &head),
            comment_ids
                .into_iter()
                .map(|id| authored(line_comment(id, &head, "request.txt", None, 1), author))
                .collect(),
        );
        deliver_review_id(&owner, review_id, GITHUB_REPOSITORY_ID).await;
    }

    assert_eq!(import_pass(&owner).await, 5);
    assert_eq!(owner.fake.permission_reads.load(Ordering::SeqCst), 4);
    let mut imported = threads(&owner)
        .await
        .iter()
        .map(|thread| thread["client_discussion_id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    imported.sort();
    assert_eq!(
        imported,
        [
            "github:review-comment:321",
            "github:review-comment:322",
            "github:review:302",
        ]
    );
    assert!(import_pass(&owner).await == 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_thread_resolved_by_someone_who_cannot_write_stays_open() {
    let owner = owner_request("review-import-threads", &[]).await;
    list_review_bots(&owner, &["greptile-apps[bot]"]).await;
    let head = owner.head();
    owner
        .fake
        .report_permission("octo-reader", Some(("read", 503)));
    owner
        .fake
        .report_permission("octo-writer", Some(("admin", 502)));
    owner.fake.report_pull_review(
        review(REVIEW_ID, bot(), &head),
        vec![line_comment(401, &head, "request.txt", None, 1)],
    );
    deliver_review(&owner, GITHUB_REPOSITORY_ID).await;
    assert_eq!(import_pass(&owner).await, 1);
    let status = |threads: Vec<serde_json::Value>| {
        by_client_id(&threads, "github:review-comment:401")["status"].clone()
    };

    owner
        .fake
        .report_review_thread(401, Some(("octo-reader", 503)));
    deliver_thread(&owner, "resolved", 401, user(503, "octo-reader", "User")).await;
    assert_eq!(import_pass(&owner).await, 1);
    assert_eq!(status(threads(&owner).await), "Open");

    owner
        .fake
        .report_review_thread(401, Some(("octo-writer", 502)));
    deliver_thread(&owner, "resolved", 401, user(502, "octo-writer", "User")).await;
    assert_eq!(import_pass(&owner).await, 1);
    let threads = threads(&owner).await;
    let resolved = by_client_id(&threads, "github:review-comment:401");
    assert_eq!(resolved["status"], "Resolved");
    assert_eq!(resolved["resolved_by"]["account"]["login"], "octo-writer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resolution_by_a_renamed_login_that_now_belongs_to_a_writer_stays_open() {
    let owner = owner_request("review-import-renamed-resolver", &[]).await;
    list_review_bots(&owner, &["greptile-apps[bot]"]).await;
    let head = owner.head();
    owner
        .fake
        .report_permission("octo-writer", Some(("admin", 502)));
    owner.fake.report_pull_review(
        review(REVIEW_ID, bot(), &head),
        vec![line_comment(402, &head, "request.txt", None, 1)],
    );
    deliver_review(&owner, GITHUB_REPOSITORY_ID).await;
    assert_eq!(import_pass(&owner).await, 1);

    owner
        .fake
        .report_review_thread(402, Some(("octo-writer", 777)));
    deliver_thread(&owner, "resolved", 402, user(777, "octo-writer", "User")).await;
    assert_eq!(import_pass(&owner).await, 1);
    let threads = threads(&owner).await;
    assert_eq!(
        by_client_id(&threads, "github:review-comment:402")["status"],
        "Open"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_late_resolution_delivery_follows_the_thread_state_on_github() {
    let owner = owner_request("review-import-late-thread", &[]).await;
    list_review_bots(&owner, &["greptile-apps[bot]"]).await;
    let head = owner.head();
    owner
        .fake
        .report_permission("octo-writer", Some(("write", 502)));
    owner.fake.report_pull_review(
        review(REVIEW_ID, bot(), &head),
        vec![line_comment(403, &head, "request.txt", None, 1)],
    );
    deliver_review(&owner, GITHUB_REPOSITORY_ID).await;
    assert_eq!(import_pass(&owner).await, 1);
    let status = |threads: Vec<serde_json::Value>| {
        by_client_id(&threads, "github:review-comment:403")["status"].clone()
    };

    owner
        .fake
        .report_permission("octo-reader", Some(("read", 503)));
    let writer = user(502, "octo-writer", "User");
    let reader = user(503, "octo-reader", "User");

    owner.fake.report_review_thread(403, None);
    deliver_thread(&owner, "resolved", 403, writer.clone()).await;
    assert_eq!(import_pass(&owner).await, 1);
    assert_eq!(status(threads(&owner).await), "Open");

    owner
        .fake
        .report_review_thread(403, Some(("octo-writer", 502)));
    deliver_thread(&owner, "resolved", 403, writer.clone()).await;
    assert_eq!(import_pass(&owner).await, 1);
    assert_eq!(status(threads(&owner).await), "Resolved");

    owner.fake.report_review_thread(403, None);
    deliver_thread(&owner, "unresolved", 403, reader).await;
    assert_eq!(import_pass(&owner).await, 1);
    deliver_thread(&owner, "resolved", 403, writer.clone()).await;
    assert_eq!(import_pass(&owner).await, 1);
    assert_eq!(status(threads(&owner).await), "Resolved");

    deliver_thread(&owner, "unresolved", 403, writer).await;
    assert_eq!(import_pass(&owner).await, 1);
    assert_eq!(status(threads(&owner).await), "Open");
}
