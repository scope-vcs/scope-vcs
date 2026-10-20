use super::*;
use crate::use_cases::github_pull_requests::run_due_github_pull_requests;

const REVIEW_COMMENT: &str = "@bot review";

async fn opted_in_request(label: &str) -> OwnerRequest {
    let request = owner_request(label, &[REQUIRED_CHECK]).await;
    let repositories = request.state.metadata.repositories();
    repositories
        .set_github_review_bot_pull_requests(TEST_REPO_ID, &test_owner_id(), true, unix_now())
        .await
        .unwrap();
    repositories
        .set_github_review_request_comment(TEST_REPO_ID, &test_owner_id(), REVIEW_COMMENT)
        .await
        .unwrap();
    request
}

async fn pull_request_pass(state: &AppState) -> usize {
    run_due_github_pull_requests(state, unix_now())
        .await
        .unwrap()
}

fn comments(fake: &FakeGitHub) -> Vec<(u64, String)> {
    fake.pulls.comments.lock().unwrap().clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pushed_request_gets_one_pull_request_commented_once_per_head_and_closed_with_it() {
    let request = opted_in_request("carrier-create").await;
    let (state, fake) = (&request.state, &request.fake);
    assert_eq!(pull_request_pass(state).await, 0);
    assert_eq!(
        request.checks().await["github_pull_request"],
        serde_json::Value::Null
    );

    push_pass(state, unix_now()).await;
    assert_eq!(
        request.checks().await["github_pull_request"]["state"],
        "opening"
    );
    assert_eq!(pull_request_pass(state).await, 1);
    let pull = fake.pulls.pull(1);
    assert_eq!(pull["head"]["ref"], request.branch());
    assert_eq!(pull["base"]["ref"], "main");
    assert_eq!(pull["draft"], false);
    assert_eq!(
        pull["title"],
        stored_request(state, &request.request_id).await.title
    );
    assert!(
        pull["body"].as_str().unwrap().contains(&format!(
            "/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/requests/{}",
            request.request_id
        )),
        "{pull}"
    );
    assert_eq!(comments(fake), vec![(1, REVIEW_COMMENT.to_string())]);
    let shown = request.checks().await["github_pull_request"].clone();
    assert_eq!(
        shown,
        serde_json::json!({
            "state": "open", "number": 1,
            "url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1"), "error": null,
        })
    );

    assert_eq!(pull_request_pass(state).await, 0);
    push_change(
        &request.source,
        &request.remote,
        "refs/heads/checks",
        "request.txt",
        "revised work\n",
        "revise request",
    )
    .unwrap();
    push_pass(state, unix_now()).await;
    assert_eq!(pull_request_pass(state).await, 1);
    assert_eq!(fake.pulls.pulls.lock().unwrap().len(), 1);
    assert_eq!(comments(fake).len(), 2);

    let closed = api_request(
        router(state.clone()),
        "DELETE",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{}", request.request_id),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(closed.status(), StatusCode::OK);
    assert_eq!(pull_request_pass(state).await, 1);
    assert_eq!(fake.pulls.pull(1)["state"], "closed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_pull_request_from_the_branch_is_adopted_instead_of_duplicated() {
    let request = opted_in_request("carrier-adopt").await;
    let (state, fake) = (&request.state, &request.fake);
    let existing = fake.pulls.open(&request.branch(), "main");

    push_pass(state, unix_now()).await;
    assert_eq!(pull_request_pass(state).await, 1);
    assert_eq!(fake.pulls.pulls.lock().unwrap().len(), 1);
    assert_eq!(comments(fake), vec![(existing, REVIEW_COMMENT.to_string())]);
    assert_eq!(
        request.checks().await["github_pull_request"]["number"],
        existing
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_pull_request_permission_gives_up_with_a_clear_error() {
    let request = opted_in_request("carrier-permission").await;
    let (state, fake) = (&request.state, &request.fake);
    fake.pulls.forbidden.store(true, Ordering::SeqCst);

    push_pass(state, unix_now()).await;
    assert_eq!(pull_request_pass(state).await, 1);
    let failed = request.checks().await["github_pull_request"].clone();
    assert_eq!(failed["state"], "failed");
    assert_eq!(
        failed["error"],
        crate::github::pull_requests::PULL_REQUEST_PERMISSION_MESSAGE
    );
    assert_eq!(pull_request_pass(state).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_the_pull_request_on_github_is_recorded_and_not_undone() {
    let request = opted_in_request("carrier-closed-on-github").await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    pull_request_pass(state).await;

    let pull = |state: &str, sender: &str| {
        serde_json::json!({
            "action": if state == "open" { "reopened" } else { "closed" },
            "repository": { "id": GITHUB_REPOSITORY_ID },
            "sender": { "login": sender },
            "pull_request": {
                "number": 1,
                "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1"),
                "state": state,
                "head": { "ref": request.branch(), "repo": { "id": GITHUB_REPOSITORY_ID } },
                "base": { "ref": "main", "repo": { "id": GITHUB_REPOSITORY_ID } },
            },
        })
    };
    fake.pulls.pulls.lock().unwrap()[0]["state"] = "closed".into();
    let delivered = webhook(
        state,
        "pull_request",
        pull("closed", "octocat"),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivered.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        request.checks().await["github_pull_request"]["state"],
        "closed_on_github"
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
    push_pass(state, unix_now()).await;
    pull_request_pass(state).await;
    assert_eq!(fake.pulls.pulls.lock().unwrap().len(), 1);
    assert_eq!(fake.pulls.pull(1)["state"], "closed");
    assert_eq!(comments(fake).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_private_request_gets_no_pull_request_once_its_github_repository_turns_public() {
    let request = opted_in_request("carrier-withheld").await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    state
        .metadata
        .repositories()
        .apply_github_repository_visibility(GITHUB_REPOSITORY_ID, false)
        .await
        .unwrap();

    assert_eq!(pull_request_pass(state).await, 1);
    assert!(fake.pulls.pulls.lock().unwrap().is_empty());
    let withheld = request.checks().await["github_pull_request"].clone();
    assert_eq!(withheld["state"], "failed");
    assert_eq!(
        withheld["error"],
        scope_domain::github_connection::PRIVATE_REQUESTS_WITHHELD_MESSAGE
    );
}
