use super::*;
use crate::use_cases::github_webhook_deliveries::process_due_github_deliveries;
use crate::workflow_tests::fake_github::queue_webhook;

async fn queue_check_run(request: &OwnerRequest, delivery_id: &str) {
    let queued = queue_webhook(
        &request.state,
        delivery_id,
        "check_run",
        serde_json::json!({
            "action": "completed",
            "repository": { "id": GITHUB_REPOSITORY_ID, "full_name": GITHUB_FULL_NAME },
            "check_run": { "id": 1, "head_sha": request.head() },
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(queued.status(), StatusCode::NO_CONTENT);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deliveries_are_answered_before_any_github_read_and_one_commit_is_read_once() {
    let request = pushed_request("deliveries-coalesced").await;
    let (state, fake) = (&request.state, &request.fake);

    for delivery_id in ["first", "second", "third", "first"] {
        queue_check_run(&request, delivery_id).await;
    }
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        request.checks().await["checks"][0]["status"],
        serde_json::Value::Null
    );

    assert_eq!(
        process_due_github_deliveries(state, unix_now())
            .await
            .unwrap(),
        3
    );
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 1);
    assert_eq!(request.checks().await["checks"][0]["status"], "in_progress");
    assert_eq!(
        process_due_github_deliveries(state, unix_now())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rate_limit_keeps_deliveries_and_stops_github_reads_until_the_reset() {
    let request = pushed_request("deliveries-rate-limited").await;
    let (state, fake) = (&request.state, &request.fake);
    let now = unix_now();
    let reset = now + 15 * 60;
    *fake.check_runs_rate_limited_until.lock().unwrap() = Some(reset);

    queue_check_run(&request, "refused").await;
    assert_eq!(process_due_github_deliveries(state, now).await.unwrap(), 1);
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 1);

    queue_check_run(&request, "arrived-while-limited").await;
    assert_eq!(process_due_github_deliveries(state, now).await.unwrap(), 1);
    assert_eq!(
        process_due_github_deliveries(state, reset - 1)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        process_due_github_deliveries(state, reset).await.unwrap(),
        2
    );
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_read_github_could_not_serve_is_retried_on_the_usual_schedule() {
    let request = pushed_request("deliveries-retried").await;
    let (state, fake) = (&request.state, &request.fake);
    let now = unix_now();
    fake.check_runs_unavailable.store(true, Ordering::SeqCst);

    queue_check_run(&request, "unserved").await;
    assert_eq!(process_due_github_deliveries(state, now).await.unwrap(), 1);
    assert_eq!(
        request.checks().await["checks"][0]["status"],
        serde_json::Value::Null
    );

    fake.check_runs_unavailable.store(false, Ordering::SeqCst);
    assert_eq!(
        process_due_github_deliveries(state, now + 29)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        process_due_github_deliveries(state, now + 30)
            .await
            .unwrap(),
        1
    );
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 2);
    assert_eq!(request.checks().await["checks"][0]["status"], "in_progress");
    assert_eq!(
        process_due_github_deliveries(state, now + 600)
            .await
            .unwrap(),
        0
    );
}
