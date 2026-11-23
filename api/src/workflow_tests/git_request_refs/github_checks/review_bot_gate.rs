use super::super::super::fake_github::webhook;
use super::*;
use crate::use_cases::{
    github::pull_requests::run_due_github_pull_requests,
    github::review_bot_statuses::reconcile_due_review_bot_statuses,
    github::review_imports::import_due_github_reviews,
};
use scope_domain::{
    github::review_bots::GitHubReviewBotInput, requests::REVIEW_BOT_START_DEADLINE_SECS,
};

const CODEX: &str = "chatgpt-codex-connector[bot]";
const GREPTILE: &str = "greptile-apps[bot]";
const AFTER_PUSH: &str = "2099-01-01T00:00:00Z";
const BEFORE_PUSH: &str = "2001-01-01T00:00:00Z";

async fn gated_request(label: &str) -> OwnerRequest {
    let request = owner_request(label, &[]).await;
    let repositories = request.state.metadata.repositories();
    repositories
        .set_github_review_bot_pull_requests(TEST_REPO_ID, &test_owner_id(), true, unix_now())
        .await
        .unwrap();
    repositories
        .set_github_review_bots(
            TEST_REPO_ID,
            &test_owner_id(),
            vec![
                GitHubReviewBotInput {
                    login: CODEX.into(),
                    required: true,
                },
                GitHubReviewBotInput {
                    login: GREPTILE.into(),
                    required: false,
                },
            ],
        )
        .await
        .unwrap();
    push_pass(&request.state, unix_now()).await;
    assert_eq!(
        run_due_github_pull_requests(&request.state, unix_now())
            .await
            .unwrap(),
        1
    );
    request
}

async fn reconcile(request: &OwnerRequest, at_unix: u64) -> usize {
    reconcile_due_review_bot_statuses(&request.state, at_unix)
        .await
        .unwrap()
}

fn codex_state(head: &str, code: &str, security: &str, findings: &str) -> String {
    codex_state_on((head, code), (head, security), findings)
}

fn codex_state_on(code: (&str, &str), security: (&str, &str), findings: &str) -> String {
    let part = |(commit, status): (&str, &str)| {
        format!(
            "{{\"commit_sha\":\"{commit}\",\"has_any_findings\":{findings},\"has_unresolved_findings\":{findings},\"status\":\"{status}\"}}"
        )
    };
    format!(
        "<!-- codex-pull-request-review-summary -->\n<!-- codex-review-state:v1 {{\"code_review\":{},\"security_review\":{}}} -->\n## Codex Review Summary",
        part(code),
        part(security)
    )
}

async fn codex_comments(request: &OwnerRequest, action: &str, body: &str) {
    request.fake.pulls.comment_as(1, CODEX, body);
    let delivered = webhook(
        &request.state,
        "issue_comment",
        serde_json::json!({
            "action": action,
            "repository": { "id": GITHUB_REPOSITORY_ID },
            "issue": { "number": 1, "pull_request": { "url": "https://api.github.com/x" } },
            "comment": { "body": body, "user": { "login": CODEX } },
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivered.status(), StatusCode::NO_CONTENT);
}

async fn bot_states(request: &OwnerRequest) -> (Vec<(String, bool, String)>, String) {
    let checks = request.checks().await;
    let bots = checks["review_bots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bot| {
            (
                bot["login"].as_str().unwrap().to_string(),
                bot["required"].as_bool().unwrap(),
                bot["state"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let status = checks["mergeability"]["status"]
        .as_str()
        .unwrap()
        .to_string();
    (bots, status)
}

fn states(codex: &str, greptile: &str) -> Vec<(String, bool, String)> {
    vec![
        (CODEX.into(), true, codex.into()),
        (GREPTILE.into(), false, greptile.into()),
    ]
}

async fn discussion_ids(request: &OwnerRequest) -> Vec<String> {
    let page = expect_json(
        api_request(
            router(request.state.clone()),
            "GET",
            &format!(
                "/v1/repos/{TEST_REPO_ID}/requests/{}/timeline",
                request.request_id
            ),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    page["discussions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|discussion| discussion["id"].as_str().unwrap().to_string())
        .collect()
}

async fn listed_status(request: &OwnerRequest) -> serde_json::Value {
    let listed = expect_json(
        api_request(
            router(request.state.clone()),
            "GET",
            &format!("/v1/repos/{TEST_REPO_ID}/requests"),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    listed["requests"][0]["mergeability"]["status"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_required_bot_holds_the_merge_until_it_finishes_the_latest_head() {
    let request = gated_request("review-bot-gate").await;
    let (fake, head) = (&request.fake, request.head());
    let now = unix_now();
    assert_eq!(
        bot_states(&request).await,
        (
            states("not_started", "not_started"),
            "ReviewsPending".into()
        )
    );
    assert_eq!(listed_status(&request).await, "ReviewsPending");

    fake.pulls.react_as(1, CODEX, "eyes", AFTER_PUSH);
    fake.pulls.react_as(1, GREPTILE, "+1", BEFORE_PUSH);
    codex_comments(
        &request,
        "created",
        &codex_state(&head, "completed", "running", "null"),
    )
    .await;
    assert_eq!(reconcile(&request, now).await, 1);
    assert_eq!(
        bot_states(&request).await,
        (states("working", "not_started"), "ReviewsPending".into())
    );

    fake.pulls.react_as(1, CODEX, "+1", AFTER_PUSH);
    assert_eq!(reconcile(&request, now + 1).await, 0);
    codex_comments(
        &request,
        "edited",
        &codex_state(&head, "completed", "completed", "false"),
    )
    .await;
    assert_eq!(reconcile(&request, now + 1).await, 1);
    assert_eq!(
        bot_states(&request).await,
        (states("clear", "not_started"), "Ready".into())
    );
    assert_eq!(listed_status(&request).await, "Ready");

    fake.pulls.review_as(1, 201, GREPTILE, &head);
    fake.pulls.rate_limited_once.store(true, Ordering::SeqCst);
    assert_eq!(reconcile(&request, now + 1_000).await, 1);
    assert_eq!(reconcile(&request, now + 2_000).await, 0);
    assert_eq!(
        bot_states(&request).await,
        (states("clear", "not_started"), "Ready".into())
    );
    request
        .state
        .metadata
        .requests()
        .requeue_github_review_bot_reconcile(GITHUB_REPOSITORY_ID, 1, now + 2_000)
        .await
        .unwrap();
    assert_eq!(reconcile(&request, now + 2_000).await, 1);
    assert_eq!(
        bot_states(&request).await,
        (states("clear", "working"), "Ready".into())
    );

    push_change(
        &request.source,
        &request.remote,
        "refs/heads/checks",
        "request.txt",
        "revised work\n",
        "revise request",
    )
    .unwrap();
    push_pass(&request.state, unix_now()).await;
    codex_comments(
        &request,
        "edited",
        &codex_state_on(
            (&request.head(), "completed"),
            (&head, "completed"),
            "false",
        ),
    )
    .await;
    reconcile_now(&request).await;
    assert_eq!(
        bot_states(&request).await,
        (states("clear", "not_started"), "Ready".into())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unresolved_bot_discussion_blocks_the_merge_until_it_is_resolved() {
    let request = gated_request("review-bot-findings").await;
    let head = request.head();
    codex_comments(
        &request,
        "created",
        &codex_state(&head, "completed", "completed", "false"),
    )
    .await;
    reconcile(&request, unix_now()).await;
    assert_eq!(
        bot_states(&request).await,
        (states("clear", "not_started"), "Ready".into())
    );

    request.fake.report_pull_review(
        serde_json::json!({
            "id": 401,
            "user": { "id": 77, "login": GREPTILE, "type": "Bot", "avatar_url": null },
            "body": "Guard this.",
            "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1#pullrequestreview-401"),
            "commit_id": head,
            "submitted_at": "2026-10-20T12:00:00Z",
        }),
        Vec::new(),
    );
    let delivered = webhook(
        &request.state,
        "pull_request_review",
        serde_json::json!({
            "action": "submitted",
            "repository": { "id": GITHUB_REPOSITORY_ID },
            "pull_request": {
                "number": 1,
                "head": { "ref": request.branch(), "repo": { "id": GITHUB_REPOSITORY_ID } },
            },
            "review": { "id": 401 },
            "sender": { "id": 77, "login": GREPTILE, "type": "Bot", "avatar_url": null },
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivered.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        import_due_github_reviews(&request.state, unix_now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        bot_states(&request).await.1,
        "ReviewFindingsOpen".to_string()
    );

    let discussion_id = discussion_ids(&request).await.remove(0);
    let resolved = api_request(
        router(request.state.clone()),
        "POST",
        &format!(
            "/v1/repos/{TEST_REPO_ID}/requests/{}/threads/{}/resolve",
            request.request_id, discussion_id
        ),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(resolved.status(), StatusCode::OK);
    assert_eq!(bot_states(&request).await.1, "Ready".to_string());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_thread_resolution_github_had_not_settled_on_reaches_its_discussion() {
    let request = gated_request("review-bot-late-resolution").await;
    let head = request.head();
    codex_comments(
        &request,
        "created",
        &codex_state(&head, "completed", "completed", "false"),
    )
    .await;
    request
        .fake
        .report_permission("octo-writer", Some(("write", 502)));
    let greptile =
        serde_json::json!({ "id": 77, "login": GREPTILE, "type": "Bot", "avatar_url": null });
    request.fake.report_pull_review(
        serde_json::json!({
            "id": 402,
            "user": greptile,
            "body": "Guard this.",
            "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1#pullrequestreview-402"),
            "commit_id": head,
            "submitted_at": "2026-10-20T12:00:00Z",
        }),
        vec![serde_json::json!({
            "id": 702,
            "in_reply_to_id": null,
            "user": greptile,
            "body": "Check the bound.",
            "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1#discussion_r702"),
            "path": "request.txt",
            "subject_type": "file",
            "commit_id": head,
            "original_commit_id": head,
            "created_at": "2026-10-20T12:00:00Z",
        })],
    );
    let deliver = async |event: &str, payload: serde_json::Value| {
        let mut payload = payload;
        payload["repository"] = serde_json::json!({ "id": GITHUB_REPOSITORY_ID });
        payload["pull_request"] = serde_json::json!({
            "number": 1,
            "head": { "ref": request.branch(), "repo": { "id": GITHUB_REPOSITORY_ID } },
        });
        let delivered = webhook(&request.state, event, payload, WEBHOOK_SECRET).await;
        assert_eq!(delivered.status(), StatusCode::NO_CONTENT);
        import_due_github_reviews(&request.state, unix_now())
            .await
            .unwrap()
    };
    let review =
        serde_json::json!({ "action": "submitted", "review": { "id": 402 }, "sender": greptile });
    assert_eq!(deliver("pull_request_review", review).await, 1);
    assert_eq!(bot_states(&request).await.1, "ReviewFindingsOpen");

    request.fake.report_review_thread(702, None);
    let resolution = serde_json::json!({
        "action": "resolved",
        "thread": { "node_id": "PRRT_702", "comments": [{ "id": 702, "in_reply_to_id": null }] },
        "sender": { "id": 502, "login": "octo-writer", "type": "User", "avatar_url": null },
    });
    assert_eq!(deliver("pull_request_review_thread", resolution).await, 1);
    assert_eq!(bot_states(&request).await.1, "ReviewFindingsOpen");

    request
        .fake
        .report_review_thread(702, Some(("octo-writer", 502)));
    reconcile_now(&request).await;
    assert_eq!(
        import_due_github_reviews(&request.state, unix_now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(bot_states(&request).await.1, "Ready");
    reconcile_now(&request).await;
    assert_eq!(
        import_due_github_reviews(&request.state, unix_now())
            .await
            .unwrap(),
        0
    );
}

fn github_time(unix: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp(i64::try_from(unix).unwrap())
        .unwrap()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}

async fn reconcile_now(request: &OwnerRequest) {
    let at_unix = unix_now();
    request
        .state
        .metadata
        .requests()
        .requeue_github_review_bot_reconcile(GITHUB_REPOSITORY_ID, 1, at_unix)
        .await
        .unwrap();
    assert_eq!(reconcile(request, at_unix).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_thumbs_up_counts_only_after_the_head_reached_github() {
    let request = gated_request("review-bot-delivery").await;
    push_change(
        &request.source,
        &request.remote,
        "refs/heads/checks",
        "request.txt",
        "revised work\n",
        "revise request",
    )
    .unwrap();
    request
        .fake
        .pulls
        .react_as(1, GREPTILE, "+1", &github_time(unix_now()));
    reconcile_now(&request).await;
    assert_eq!(
        bot_states(&request).await.0,
        states("not_started", "not_started")
    );

    push_pass(&request.state, unix_now()).await;
    let delivered_at = request
        .state
        .metadata
        .requests()
        .github_head_delivered_at(&request.request_id, &request.head())
        .await
        .unwrap()
        .unwrap();
    request
        .fake
        .pulls
        .react_as(1, GREPTILE, "+1", &github_time(delivered_at));
    reconcile_now(&request).await;
    assert_eq!(
        bot_states(&request).await.0,
        states("not_started", "not_started")
    );

    request.fake.pulls.react_as(1, GREPTILE, "+1", AFTER_PUSH);
    reconcile_now(&request).await;
    assert_eq!(bot_states(&request).await.0, states("not_started", "clear"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_thumbs_up_on_the_comment_that_asked_for_the_review_clears_the_bot() {
    let request = gated_request("review-bot-comment-reaction").await;
    let pulls = &request.fake.pulls;
    for _ in 0..12 {
        let earlier = pulls.request_review_as(1, "octocat", "@greptileai review");
        pulls.react_to_comment_as(earlier, "octocat", "+1", AFTER_PUSH);
    }
    let asked = pulls.request_review_as(1, "octocat", "@greptileai review");
    pulls.react_to_comment_as(asked, "octocat", "+1", AFTER_PUSH);
    pulls.react_to_comment_as(asked, GREPTILE, "+1", BEFORE_PUSH);
    reconcile_now(&request).await;
    assert_eq!(
        bot_states(&request).await.0,
        states("not_started", "not_started")
    );

    pulls.react_to_comment_as(asked, GREPTILE, "eyes", AFTER_PUSH);
    reconcile_now(&request).await;
    assert_eq!(
        bot_states(&request).await.0,
        states("not_started", "working")
    );

    pulls.react_to_comment_as(asked, GREPTILE, "+1", AFTER_PUSH);
    reconcile_now(&request).await;
    assert_eq!(bot_states(&request).await.0, states("not_started", "clear"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_findings_hold_the_merge_until_their_review_is_imported() {
    let request = gated_request("review-bot-import").await;
    let head = request.head();
    request.fake.report_pull_review(
        serde_json::json!({
            "id": 301,
            "user": { "id": 88, "login": CODEX, "type": "Bot", "avatar_url": null },
            "body": "Guard the empty list.",
            "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1#pullrequestreview-301"),
            "commit_id": head,
            "submitted_at": "2026-10-20T12:00:00Z",
        }),
        Vec::new(),
    );
    request.fake.pulls.review_as(1, 301, CODEX, &head);
    codex_comments(
        &request,
        "edited",
        &codex_state(&head, "completed", "completed", "true"),
    )
    .await;
    reconcile_now(&request).await;
    assert_eq!(
        bot_states(&request).await,
        (states("working", "not_started"), "ReviewsPending".into())
    );

    assert_eq!(
        import_due_github_reviews(&request.state, unix_now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(reconcile(&request, unix_now()).await, 1);
    assert_eq!(
        bot_states(&request).await,
        (
            states("commented", "not_started"),
            "ReviewFindingsOpen".into()
        )
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_newest_reactions_are_read_past_a_thousand_older_ones() {
    let request = gated_request("review-bot-newest").await;
    request
        .fake
        .pulls
        .react_many_as(1, "octocat", 1_150, BEFORE_PUSH);
    request.fake.pulls.react_as(1, GREPTILE, "+1", AFTER_PUSH);
    reconcile_now(&request).await;
    assert_eq!(bot_states(&request).await.0, states("not_started", "clear"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn required_bots_hold_the_merge_only_while_review_bot_pull_requests_are_on() {
    let request = gated_request("review-bot-carriers-off").await;
    let checks = request.checks().await;
    assert_eq!(checks["review_bots_enforced"], true);
    assert_eq!(checks["mergeability"]["status"], "ReviewsPending");

    request
        .state
        .metadata
        .repositories()
        .set_github_review_bot_pull_requests(TEST_REPO_ID, &test_owner_id(), false, unix_now())
        .await
        .unwrap();
    let checks = request.checks().await;
    assert_eq!(checks["review_bots_enforced"], false);
    assert_eq!(checks["review_bots"][0]["required"], true);
    assert_eq!(checks["mergeability"]["status"], "Ready");
    assert_eq!(listed_status(&request).await, "Ready");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_required_bot_that_never_starts_after_the_review_request_turns_advisory() {
    let request = gated_request("review-bot-silent").await;
    let state = &request.state;
    state
        .metadata
        .repositories()
        .set_github_review_request_comment(TEST_REPO_ID, &test_owner_id(), "@codex review")
        .await
        .unwrap();
    let requested_after = unix_now();
    state
        .metadata
        .requests()
        .requeue_github_pull_request(GITHUB_REPOSITORY_ID, &request.request_id, requested_after)
        .await
        .unwrap();
    assert_eq!(
        run_due_github_pull_requests(state, requested_after)
            .await
            .unwrap(),
        1
    );
    let requested_before = unix_now();

    assert_eq!(
        reconcile(
            &request,
            requested_after + REVIEW_BOT_START_DEADLINE_SECS - 1
        )
        .await,
        1
    );
    assert_eq!(
        bot_states(&request).await,
        (
            states("not_started", "not_started"),
            "ReviewsPending".into()
        )
    );

    request
        .state
        .metadata
        .requests()
        .requeue_github_review_bot_reconcile(
            GITHUB_REPOSITORY_ID,
            1,
            requested_before + REVIEW_BOT_START_DEADLINE_SECS,
        )
        .await
        .unwrap();
    assert_eq!(
        reconcile(&request, requested_before + REVIEW_BOT_START_DEADLINE_SECS).await,
        1
    );
    assert_eq!(
        bot_states(&request).await,
        (states("timed_out", "timed_out"), "Ready".into())
    );
    assert_eq!(listed_status(&request).await, "Ready");
}
