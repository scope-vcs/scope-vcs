//! Connected GitHub repositories that are public, or become public.

use super::*;

/// GitHub now lists the connected repository as public.
pub(super) fn make_github_repository_public(fake: &FakeGitHub) {
    let mut public = github_repository(GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME);
    public["private"] = serde_json::json!(false);
    *fake.installation_repositories.lock().unwrap() = vec![(INSTALLATION_ID, public)];
}

async fn github_settings(state: &AppState) -> serde_json::Value {
    expect_json(
        api_request(
            router(state.clone()),
            "GET",
            "/v1/repos/owner/repo/github",
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_private_request_is_not_sent_to_a_repository_that_became_public_until_confirmed() {
    let request = owner_request("github-checks-became-public", &[REQUIRED_CHECK]).await;
    let (state, fake) = (&request.state, &request.fake);
    make_github_repository_public(fake);

    // No delivery said so; the push asks GitHub first and holds the request.
    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), None);
    let held = request.checks().await;
    assert_eq!(held["github_push"]["state"], "failed");
    assert_eq!(held["mergeability"]["status"], "ChecksConfigurationError");
    assert_eq!(
        held["message"],
        scope_domain::github_connection::PRIVATE_REQUESTS_WITHHELD_MESSAGE
    );
    assert_eq!(held["private_request_on_public_github"], true);
    let settings = github_settings(state).await;
    assert_eq!(settings["connection"]["public_on_github"], true);
    assert_eq!(settings["connection"]["public_confirmed"], false);

    // A maintainer who can change file visibility confirms; the request goes.
    let confirmed = expect_json(
        api_request(
            router(state.clone()),
            "POST",
            "/v1/repos/owner/repo/github/public-confirmation",
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(confirmed["connection"]["public_confirmed"], true);
    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), Some(request.head()));
    assert_eq!(
        request.checks().await["mergeability"]["status"],
        "ChecksPending"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delivery_saying_the_repository_became_public_holds_private_requests() {
    let request = owner_request("github-checks-publicized", &[REQUIRED_CHECK]).await;
    let (state, fake) = (&request.state, &request.fake);
    // A late delivery is checked against what GitHub says now.
    let publicized = serde_json::json!({
        "action": "publicized",
        "repository": { "id": GITHUB_REPOSITORY_ID, "full_name": GITHUB_FULL_NAME },
    });
    let delivery = webhook(state, "repository", publicized.clone(), WEBHOOK_SECRET).await;
    assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        github_settings(state).await["connection"]["public_on_github"],
        false
    );

    make_github_repository_public(fake);
    let delivery = webhook(state, "repository", publicized, WEBHOOK_SECRET).await;
    assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        github_settings(state).await["connection"]["public_confirmed"],
        false
    );
    assert_eq!(
        request.checks().await["mergeability"]["status"],
        "ChecksConfigurationError"
    );
    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), None);
}
