use super::*;
use crate::workflow_tests::fake_github::check_sources::RepositoryReadRefusal;
use scope_domain::requests::{GitHubPush, GitHubPushState};

async fn latest_push(request: &OwnerRequest) -> GitHubPush {
    request
        .state
        .metadata
        .requests()
        .request_state_snapshot(
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            &request.request_id,
            Some(&test_owner_id()),
        )
        .await
        .unwrap()
        .github_push
        .unwrap()
}

fn refuse_next_repository_read(request: &OwnerRequest, refusal: RepositoryReadRefusal) {
    *request
        .fake
        .check_sources
        .repository_read_refusal
        .lock()
        .unwrap() = Some(refusal);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rate_limited_push_waits_for_the_reset_and_keeps_its_attempts() {
    let request = owner_request("push-rate-limited", &[REQUIRED_CHECK]).await;
    let (state, fake) = (&request.state, &request.fake);
    let now = unix_now();
    let reset = now + 17 * 60;
    refuse_next_repository_read(&request, RepositoryReadRefusal::RateLimitedUntil(reset));

    assert_eq!(push_pass(state, now).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), None);
    let waiting = latest_push(&request).await;
    assert_eq!(waiting.state, GitHubPushState::Queued);
    assert_eq!(waiting.attempts, 0);
    assert_eq!(
        waiting.last_error.as_deref(),
        Some(crate::github::RATE_LIMIT_MESSAGE)
    );
    fake.client_clock_unix.store(reset - 1, Ordering::SeqCst);
    assert_eq!(push_pass(state, reset - 1).await, 0);

    fake.client_clock_unix.store(reset, Ordering::SeqCst);
    assert_eq!(push_pass(state, reset).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), Some(request.head()));
    let sent = latest_push(&request).await;
    assert_eq!(sent.state, GitHubPushState::Succeeded);
    assert_eq!(sent.attempts, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_push_github_could_not_serve_backs_off_and_counts_the_attempt() {
    let request = owner_request("push-unavailable", &[REQUIRED_CHECK]).await;
    let (state, fake) = (&request.state, &request.fake);
    let now = unix_now();
    refuse_next_repository_read(&request, RepositoryReadRefusal::Unavailable);

    assert_eq!(push_pass(state, now).await, 1);
    let retrying = latest_push(&request).await;
    assert_eq!(retrying.state, GitHubPushState::Queued);
    assert_eq!(retrying.attempts, 1);
    assert!(
        retrying
            .last_error
            .as_deref()
            .is_some_and(|error| error.starts_with("GitHub could not complete the request.")),
        "{:?}",
        retrying.last_error
    );
    assert_eq!(fake.branch_head(&request.branch()), None);

    assert_eq!(push_pass(state, now + 60).await, 1);
    assert_eq!(latest_push(&request).await.attempts, 2);
    assert_eq!(fake.branch_head(&request.branch()), Some(request.head()));
}
