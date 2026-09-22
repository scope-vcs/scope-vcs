use super::*;
use axum::http::StatusCode;
use serde_json::json;

#[tokio::test]
async fn start_sends_only_attempt_and_bootstrap_to_the_broker() {
    let provider = fake::FakeEcs::new().await;
    provider.starts.add_permits(1);
    assert_eq!(
        provider
            .client
            .start("attempt_1", "scope_bootstrap_test")
            .await
            .unwrap(),
        "task-attempt_1"
    );
    assert_eq!(
        provider.request_body("start"),
        json!({"action": "start", "attempt_id": "attempt_1", "bootstrap_token": "scope_bootstrap_test"})
    );
}

#[tokio::test]
async fn only_explicit_broker_rejection_is_a_safe_setup_failure() {
    let provider = fake::FakeEcs::new().await;
    provider.reply(
        StatusCode::OK,
        json!({"status":"rejected", "reason":"authorization", "message":"authorization denied"}),
        false,
    );
    assert!(matches!(
        provider.client.start("attempt_1", "token").await,
        Err(StartError::Rejected {
            reason: RejectionReason::Authorization,
            ..
        })
    ));
    for (status, reply, function_error) in [
        (
            StatusCode::OK,
            json!({"status":"ambiguous", "message":"launch response lost"}),
            false,
        ),
        (
            StatusCode::OK,
            json!({"status":"started", "task_arn":""}),
            false,
        ),
        (StatusCode::OK, json!({"status":"stopped"}), false),
        (
            StatusCode::OK,
            json!({"status":"rejected", "reason":"authorization", "message":"denied", "extra":true}),
            false,
        ),
        (
            StatusCode::OK,
            json!({"status":"rejected", "reason":"authorization", "message":"denied"}),
            true,
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"message":"failure"}),
            false,
        ),
        (StatusCode::FORBIDDEN, json!({"message":"denied"}), false),
    ] {
        provider.reply(status, reply, function_error);
        assert!(matches!(
            provider.client.start("attempt_1", "token").await,
            Err(StartError::Ambiguous(_))
        ));
    }
}

#[tokio::test]
async fn retry_is_unblocked_only_after_broker_confirms_cleanup() {
    let provider = fake::FakeEcs::new().await;
    for reply in [
        json!({"status":"ambiguous", "message":"still stopping"}),
        json!({"status":"rejected", "reason":"authorization", "message":"not authorized"}),
        json!({"status":"started", "task_arn":"task-1"}),
    ] {
        provider.reply(StatusCode::OK, reply, false);
        assert!(
            provider
                .client
                .stop_terminal_task("attempt_1")
                .await
                .is_err()
        );
    }
    let provider = fake::FakeEcs::new().await;
    provider.stops.add_permits(1);
    provider
        .client
        .stop_terminal_task("attempt_1")
        .await
        .map(|outcome| assert_eq!(outcome, StopOutcome::Stopped))
        .unwrap();
    assert_eq!(
        provider.request_body("stop"),
        json!({"action":"stop", "attempt_id":"attempt_1"})
    );
}

#[tokio::test]
async fn stop_progress_is_explicit_and_cannot_confirm_cleanup() {
    let provider = fake::FakeEcs::new().await;
    provider.reply(
        StatusCode::OK,
        json!({"status":"stopping", "stuck":false}),
        false,
    );
    assert_eq!(
        provider
            .client
            .stop_terminal_task("attempt_1")
            .await
            .unwrap(),
        StopOutcome::Stopping { stuck: false }
    );
    provider.reply(
        StatusCode::OK,
        json!({"status":"stopping", "stuck":true}),
        false,
    );
    assert_eq!(
        provider
            .client
            .stop_terminal_task("attempt_1")
            .await
            .unwrap(),
        StopOutcome::Stopping { stuck: true }
    );
    provider.reply(StatusCode::OK, json!({"status":"stopping"}), false);
    assert!(
        provider
            .client
            .stop_terminal_task("attempt_1")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn capacity_and_quota_rejections_keep_distinct_reasons() {
    let provider = fake::FakeEcs::new().await;
    for (wire, expected) in [
        ("capacity", RejectionReason::Capacity),
        ("quota", RejectionReason::Quota),
        ("permanent", RejectionReason::Permanent),
    ] {
        provider.reply(
            StatusCode::OK,
            json!({"status":"rejected", "reason":wire, "message":"ECS rejected this attempt"}),
            false,
        );
        assert!(
            matches!(provider.client.start("attempt_1", "token").await, Err(StartError::Rejected { reason, .. }) if reason == expected)
        );
    }
    provider.reply(
        StatusCode::OK,
        json!({"status":"rejected", "reason":"unknown", "message":"denied"}),
        false,
    );
    assert!(matches!(
        provider.client.start("attempt_1", "token").await,
        Err(StartError::Ambiguous(_))
    ));
}

#[tokio::test]
async fn lost_broker_response_is_ambiguous_and_never_retried() {
    let provider = fake::FakeEcs::with_timeout(Duration::from_millis(100)).await;
    assert!(matches!(
        provider.client.start("attempt_timeout", "token").await,
        Err(StartError::Ambiguous(_))
    ));
    assert_eq!(provider.count("start"), 1);
}
