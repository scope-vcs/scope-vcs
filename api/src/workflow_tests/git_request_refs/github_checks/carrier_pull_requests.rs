use super::*;
use crate::use_cases::github_pull_requests::run_due_github_pull_requests;
use scope_domain::requests::review_request_comment_body;

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

fn review_request(head: &str) -> String {
    review_request_comment_body(REVIEW_COMMENT, head)
}

async fn close_request(request: &OwnerRequest) {
    let closed = api_request(
        router(request.state.clone()),
        "DELETE",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{}", request.request_id),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(closed.status(), StatusCode::OK);
}

async fn retitle(request: &OwnerRequest, title: &str, description: &str) {
    let edited = api_request(
        router(request.state.clone()),
        "PATCH",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{}", request.request_id),
        Some(&bearer_header()),
        Some(
            &serde_json::json!({ "title": title, "description_markdown": description }).to_string(),
        ),
    )
    .await;
    assert_eq!(edited.status(), StatusCode::OK);
}

async fn revise(request: &OwnerRequest, contents: &str) {
    push_change(
        &request.source,
        &request.remote,
        "refs/heads/checks",
        "request.txt",
        contents,
        "revise request",
    )
    .unwrap();
    push_pass(&request.state, unix_now()).await;
}

fn pull_request_delivery(request: &OwnerRequest, action: &str, state: &str) -> serde_json::Value {
    serde_json::json!({
        "action": action,
        "repository": { "id": GITHUB_REPOSITORY_ID },
        "sender": { "login": "octocat" },
        "pull_request": {
            "number": 1,
            "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1"),
            "state": state,
            "head": { "ref": request.branch(), "repo": { "id": GITHUB_REPOSITORY_ID } },
            "base": { "ref": "main", "repo": { "id": GITHUB_REPOSITORY_ID } },
        },
    })
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
    assert_eq!(comments(fake), vec![(1, review_request(&request.head()))]);
    let shown = request.checks().await["github_pull_request"].clone();
    assert_eq!(
        shown,
        serde_json::json!({
            "state": "open", "number": 1,
            "url": format!("https://github.com/{GITHUB_FULL_NAME}/pull/1"), "error": null,
        })
    );

    assert_eq!(pull_request_pass(state).await, 0);
    revise(&request, "revised work\n").await;
    assert_eq!(pull_request_pass(state).await, 1);
    assert_eq!(fake.pulls.pulls.lock().unwrap().len(), 1);
    assert_eq!(comments(fake).len(), 2);

    close_request(&request).await;
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
    assert_eq!(
        comments(fake),
        vec![(existing, review_request(&request.head()))]
    );
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
async fn a_close_on_github_wins_over_a_stale_opened_delivery_and_is_not_undone() {
    let request = opted_in_request("carrier-closed-on-github").await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    pull_request_pass(state).await;

    fake.pulls.close_as(1, "octocat");
    for (action, payload_state) in [("closed", "closed"), ("opened", "open")] {
        let delivered = webhook(
            state,
            "pull_request",
            pull_request_delivery(&request, action, payload_state),
            WEBHOOK_SECRET,
        )
        .await;
        assert_eq!(delivered.status(), StatusCode::NO_CONTENT);
    }
    assert_eq!(pull_request_pass(state).await, 1);
    assert_eq!(
        request.checks().await["github_pull_request"]["state"],
        "closed_on_github"
    );

    revise(&request, "revised work\n").await;
    pull_request_pass(state).await;
    assert_eq!(fake.pulls.pulls.lock().unwrap().len(), 1);
    assert_eq!(fake.pulls.pull(1)["state"], "closed");
    assert_eq!(comments(fake).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turning_the_setting_off_stops_a_queued_pull_request() {
    let request = opted_in_request("carrier-turned-off").await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    state
        .metadata
        .repositories()
        .set_github_review_bot_pull_requests(TEST_REPO_ID, &test_owner_id(), false, unix_now())
        .await
        .unwrap();

    assert_eq!(pull_request_pass(state).await, 1);
    assert!(fake.pulls.pulls.lock().unwrap().is_empty());
    assert!(comments(fake).is_empty());
    assert_eq!(
        request.checks().await["github_pull_request"],
        serde_json::Value::Null
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rate_limit_waits_for_its_reset_instead_of_failing() {
    let request = opted_in_request("carrier-rate-limited").await;
    let (state, fake) = (&request.state, &request.fake);
    fake.pulls.rate_limited_once.store(true, Ordering::SeqCst);
    push_pass(state, unix_now()).await;

    assert_eq!(pull_request_pass(state).await, 1);
    assert!(fake.pulls.pulls.lock().unwrap().is_empty());
    let waiting = request.checks().await["github_pull_request"].clone();
    assert_eq!(waiting["state"], "opening");
    assert!(
        waiting["error"].as_str().unwrap().contains("rate limit"),
        "{waiting}"
    );
    assert_eq!(pull_request_pass(state).await, 0);
    assert_eq!(
        run_due_github_pull_requests(state, 4_102_444_800)
            .await
            .unwrap(),
        1
    );
    assert_eq!(fake.pulls.pulls.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_counts_only_when_github_confirms_it() {
    let request = opted_in_request("carrier-unreachable-close").await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    pull_request_pass(state).await;

    close_request(&request).await;
    fake.pulls.writes_unreachable.store(true, Ordering::SeqCst);
    assert_eq!(pull_request_pass(state).await, 1);
    assert_eq!(fake.pulls.pull(1)["state"], "open");
    let retrying = request.checks().await["github_pull_request"].clone();
    assert_eq!(retrying["state"], "open");
    assert_eq!(
        retrying["error"],
        "The Scope GitHub App cannot reach this pull request on GitHub."
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_review_request_already_on_github_for_the_head_is_not_posted_again() {
    let request = opted_in_request("carrier-comment-once").await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    pull_request_pass(state).await;
    assert_eq!(comments(fake).len(), 1);

    revise(&request, "revised work\n").await;
    fake.pulls
        .comment_as_app(1, &review_request(&request.head()));
    assert_eq!(pull_request_pass(state).await, 1);
    assert_eq!(comments(fake).len(), 2);
    assert_eq!(pull_request_pass(state).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_private_request_gets_no_pull_request_when_github_turned_public_before_the_webhook() {
    let request = opted_in_request("carrier-unannounced-public").await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    for (_, repository) in fake.installation_repositories.lock().unwrap().iter_mut() {
        repository["private"] = false.into();
    }

    assert_eq!(pull_request_pass(state).await, 1);
    assert!(fake.pulls.pulls.lock().unwrap().is_empty());
    assert!(comments(fake).is_empty());
    let withheld = request.checks().await["github_pull_request"].clone();
    assert_eq!(withheld["state"], "failed");
    assert_eq!(
        withheld["error"],
        scope_domain::github_connection::PRIVATE_REQUESTS_WITHHELD_MESSAGE
    );
    assert_eq!(
        request.checks().await["private_request_on_public_github"],
        true
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn editing_the_request_rewrites_the_title_and_body_scope_wrote() {
    let request = opted_in_request("carrier-retitled").await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    pull_request_pass(state).await;

    retitle(&request, "Parser fix, take two", "Now with tests.").await;
    assert_eq!(pull_request_pass(state).await, 1);
    let pull = fake.pulls.pull(1);
    assert_eq!(pull["title"], "Parser fix, take two");
    let body = pull["body"].as_str().unwrap();
    assert!(
        body.starts_with("Review copy of [this Scope request]("),
        "{body}"
    );
    assert!(body.ends_with("\n\nNow with tests."), "{body}");
    assert_eq!(pull_request_pass(state).await, 0);
    assert_eq!(comments(fake).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_adopted_pull_request_keeps_the_title_and_body_its_author_wrote() {
    let request = opted_in_request("carrier-adopted-text").await;
    let (state, fake) = (&request.state, &request.fake);
    let adopted = fake.pulls.open(&request.branch(), "main");
    fake.pulls.pulls.lock().unwrap()[0]["title"] = "Written by hand".into();
    push_pass(state, unix_now()).await;
    pull_request_pass(state).await;

    retitle(&request, "Scope title", "Scope description").await;
    pull_request_pass(state).await;
    assert_eq!(fake.pulls.pull(adopted)["title"], "Written by hand");
    assert_eq!(fake.pulls.pull(adopted)["body"], "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_close_delivered_before_the_first_run_saved_the_pull_request_still_wins() {
    let request = opted_in_request("carrier-early-close").await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    let requests = state.metadata.requests();
    let now = unix_now();
    let mut opening = requests
        .claim_due_github_pull_requests("first_run", now, now + 300, 1)
        .await
        .unwrap()
        .remove(0);
    let number = fake.pulls.open(&request.branch(), "main");
    opening.record_opened(
        number,
        format!("https://github.com/{GITHUB_FULL_NAME}/pull/{number}"),
        "main".into(),
        None,
    );

    fake.pulls.close_as(number, "octocat");
    let delivered = webhook(
        state,
        "pull_request",
        pull_request_delivery(&request, "closed", "closed"),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivered.status(), StatusCode::NO_CONTENT);
    assert!(
        requests
            .finish_github_pull_request(
                &opening,
                "first_run",
                scope_postgres::db::GitHubJobOutcome::Succeeded(()),
                now
            )
            .await
            .unwrap()
    );

    assert_eq!(pull_request_pass(state).await, 1);
    assert_eq!(
        request.checks().await["github_pull_request"]["state"],
        "closed_on_github"
    );
    revise(&request, "revised work\n").await;
    pull_request_pass(state).await;
    assert!(comments(fake).is_empty());
    assert_eq!(fake.pulls.pulls.lock().unwrap().len(), 1);
}
