use super::*;
use crate::use_cases::github_webhook_deliveries::process_due_github_deliveries;
use crate::workflow_tests::fake_github::queue_webhook;
use std::sync::atomic::AtomicU64;

fn check_run_delivery(head_sha: &str) -> serde_json::Value {
    serde_json::json!({
        "action": "completed",
        "repository": { "id": GITHUB_REPOSITORY_ID, "full_name": GITHUB_FULL_NAME },
        "check_run": { "id": 1, "head_sha": head_sha },
    })
}

async fn queue_check_run(request: &OwnerRequest, delivery_id: &str) {
    let queued = queue_webhook(
        &request.state,
        delivery_id,
        "check_run",
        check_run_delivery(&request.head()),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(queued.status(), StatusCode::NO_CONTENT);
}

async fn process_at(state: &AppState, now_unix: u64) -> usize {
    process_due_github_deliveries(state, || Ok(now_unix))
        .await
        .unwrap()
}

async fn pushed_request(label: &str) -> OwnerRequest {
    let request = owner_request(label, &[REQUIRED_CHECK]).await;
    assert_eq!(push_pass(&request.state, unix_now()).await, 1);
    let head = request.head();
    request
        .fake
        .report_check_runs(&head, vec![check_run(1, REQUIRED_CHECK, &head, None)]);
    request
}

async fn first_check_status(request: &OwnerRequest) -> serde_json::Value {
    request.checks().await["checks"][0]["status"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deliveries_are_answered_before_any_github_read_and_one_commit_is_read_once() {
    let request = pushed_request("deliveries-coalesced").await;
    let (state, fake) = (&request.state, &request.fake);

    for delivery_id in ["first", "second", "third", "first"] {
        queue_check_run(&request, delivery_id).await;
    }
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 0);
    assert_eq!(first_check_status(&request).await, serde_json::Value::Null);

    assert_eq!(process_at(state, unix_now()).await, 3);
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 1);
    assert_eq!(first_check_status(&request).await, "in_progress");
    assert_eq!(process_at(state, unix_now()).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rate_limit_keeps_deliveries_stops_github_reads_and_resumes_at_the_reset() {
    let request = pushed_request("deliveries-rate-limited").await;
    let (state, fake) = (&request.state, &request.fake);
    let now = unix_now();
    let reset = now + 15 * 60;
    *fake.check_runs_rate_limited_until.lock().unwrap() = Some(reset);

    queue_check_run(&request, "refused").await;
    assert_eq!(process_at(state, unix_now()).await, 1);
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 1);

    queue_check_run(&request, "arrived-while-limited").await;
    assert_eq!(process_at(state, unix_now()).await, 1);
    assert_eq!(process_at(state, reset - 1).await, 0);
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 1);
    assert_eq!(first_check_status(&request).await, serde_json::Value::Null);

    *fake.check_runs_rate_limited_until.lock().unwrap() = None;
    fake.client_clock_unix.store(reset, Ordering::SeqCst);
    assert_eq!(process_at(state, reset).await, 2);
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 2);
    assert_eq!(first_check_status(&request).await, "in_progress");
    assert_eq!(process_at(state, reset + 3_600).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_read_github_could_not_serve_is_retried_on_the_usual_schedule() {
    let request = pushed_request("deliveries-retried").await;
    let (state, fake) = (&request.state, &request.fake);
    fake.check_runs_unavailable.store(true, Ordering::SeqCst);

    queue_check_run(&request, "unserved").await;
    let now = unix_now();
    assert_eq!(process_at(state, now).await, 1);
    assert_eq!(first_check_status(&request).await, serde_json::Value::Null);

    fake.check_runs_unavailable.store(false, Ordering::SeqCst);
    assert_eq!(process_at(state, now + 29).await, 0);
    assert_eq!(process_at(state, now + 30).await, 1);
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 2);
    assert_eq!(first_check_status(&request).await, "in_progress");
    assert_eq!(process_at(state, now + 600).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_slower_than_the_lease_gives_unstarted_deliveries_back_without_spending_attempts() {
    let request = pushed_request("deliveries-slow").await;
    let (state, fake) = (&request.state, &request.fake);
    let repositories = state.metadata.repositories();
    let last = check_run_delivery(&request.head()).to_string();
    for (delivery_id, head_sha, received_at_unix) in [
        ("first", "1".repeat(40), 1),
        ("second", "2".repeat(40), 2),
        ("last", request.head(), 3),
    ] {
        let payload = check_run_delivery(&head_sha).to_string();
        repositories
            .queue_github_webhook_delivery(
                delivery_id,
                "check_run",
                payload.as_bytes(),
                received_at_unix,
            )
            .await
            .unwrap();
    }
    fake.check_runs_unavailable.store(true, Ordering::SeqCst);
    let clock = AtomicU64::new(1_000);
    let failed_at = 3_000;
    let slower_than_the_lease = || {
        Ok(clock
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |now| {
                Some((now + 200).min(failed_at))
            })
            .unwrap_or(failed_at))
    };

    assert_eq!(
        process_due_github_deliveries(state, slower_than_the_lease)
            .await
            .unwrap(),
        3
    );
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 1);

    let claim = async |now_unix| {
        repositories
            .claim_due_github_webhook_deliveries(now_unix, now_unix + 300, 10)
            .await
            .unwrap()
    };
    assert_eq!(claim(failed_at + 29).await, []);
    let retried = claim(failed_at + 30).await;
    assert_eq!(retried.len(), 1);
    assert_eq!(
        (retried[0].delivery_id.as_str(), retried[0].attempts),
        ("last", 2)
    );
    assert_eq!(retried[0].payload, last.as_bytes());
}
