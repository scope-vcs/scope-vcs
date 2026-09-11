use super::*;
use scope_api_contract::RunChangeKind;
use std::time::Duration;
use tokio_stream::StreamExt;

async fn subscribe(fixture: &InspectableRun) -> axum::body::BodyDataStream {
    let path = scope_api_contract::routes::repo_run_events(
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &fixture.run_id,
    );
    let response = api_request(
        router(fixture.state.clone()),
        "GET",
        &format!("{path}?after={}", fixture.second_log_position),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response.into_body().into_data_stream()
}

async fn next_status(stream: &mut axum::body::BodyDataStream) -> serde_json::Value {
    let frame = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("a changed status must be sent without waiting for reconciliation")
        .expect("stream must remain open while its attempt is running")
        .unwrap();
    let text = std::str::from_utf8(&frame).unwrap();
    assert!(text.contains("event: status"), "{text}");
    let data = text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    serde_json::from_str(data).unwrap()
}

#[tokio::test]
async fn cancellation_is_streamed_while_the_worker_keeps_the_run_running() {
    let fixture = active_inspectable_run(false).await;
    let mut stream = subscribe(&fixture).await;
    let initial = next_status(&mut stream).await;
    assert_eq!(initial["state"], "running");
    assert_eq!(initial["cancellation_requested"], false);

    let cancelled = api_request(
        router(fixture.state.clone()),
        "POST",
        &scope_api_contract::routes::repo_run_cancel(
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            &fixture.run_id,
        ),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(cancelled.status(), StatusCode::OK);
    let response = response_json(cancelled).await;
    assert_eq!(response["state"], "running");
    assert_eq!(response["cancellation_requested"], true);

    let changed = next_status(&mut stream).await;
    assert_eq!(changed["state"], "running");
    assert_eq!(changed["cancellation_requested"], true);
    assert!(changed["completed_at_unix"].is_null());
}

#[tokio::test]
async fn log_truncation_is_streamed_before_the_attempt_finishes() {
    let fixture = active_inspectable_run(false).await;
    let mut stream = subscribe(&fixture).await;
    let initial = next_status(&mut stream).await;
    assert_eq!(initial["state"], "running");
    assert_eq!(initial["logs_truncated"], false);

    fixture
        .state
        .metadata
        .runs()
        .complete_attempt_step(
            &fixture.attempt_id,
            &"d".repeat(64),
            0,
            StepConclusion::Succeeded,
            true,
            7,
        )
        .await
        .unwrap();
    fixture
        .state
        .publish_run_change(
            TEST_REPO_ID,
            fixture.run_id.clone(),
            RunChangeKind::StatusChanged,
        )
        .await;

    let changed = next_status(&mut stream).await;
    assert_eq!(changed["state"], "running");
    assert_eq!(changed["logs_truncated"], true);
    assert!(changed["completed_at_unix"].is_null());
}
