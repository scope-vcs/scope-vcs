use super::tests::queued_runs;
use super::*;
use crate::execution::fake::FakeEcs;
use scope_domain::runs::{
    run::RunState,
    step::{AttemptTerminalReason, MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES},
};
use serde_json::json;

#[tokio::test]
async fn multibyte_capacity_rejection_records_retry() {
    let metadata = queued_runs(1).await;
    let provider = FakeEcs::new().await;
    provider.reply(
        axum::http::StatusCode::OK,
        json!({"status":"rejected","reason":"capacity","message":"é".repeat(2048)}),
        false,
    );
    let execution = coordinator(metadata.clone(), &provider);
    let now = crate::unix_now().unwrap();

    assert_eq!(execution.dispatch_available(now).await.unwrap(), 1);
    assert_eq!(
        metadata.runs().run("run-0").await.unwrap().unwrap().state,
        RunState::Queued
    );
    let jobs = metadata.runs().run_jobs("run-0").await.unwrap();
    let retry = jobs[0].capacity_retry.as_ref().unwrap();
    assert_eq!(retry.rejections, 1);
    assert!(retry.next_attempt_at_unix.unwrap() > now);
    let detail = metadata.runs().run_detail("run-0").await.unwrap().unwrap();
    let Some(AttemptTerminalReason::ProviderCapacityRejected { message }) =
        &detail.attempts[0].attempt.terminal_reason
    else {
        panic!("expected a recorded capacity rejection");
    };
    assert!(
        message
            .as_str()
            .starts_with("provider rejected dispatch: é")
    );
    assert!(message.as_str().len() <= MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES);
}

fn coordinator(metadata: MetadataStore, provider: &FakeEcs) -> CloudExecutionCoordinator {
    CloudExecutionCoordinator {
        metadata,
        product_analytics: ProductAnalytics::disabled(),
        ecs: provider.client.clone(),
        origin_id: "capacity-test".into(),
        settings: provider.settings(),
    }
}

#[tokio::test]
async fn capacity_rejection_retries_after_restart_and_stops_after_three_extra_attempts() {
    let metadata = queued_runs(1).await;
    let provider = FakeEcs::new().await;
    provider.reply(
        axum::http::StatusCode::OK,
        json!({"status":"rejected","reason":"capacity","message":"capacity unavailable"}),
        false,
    );
    let first = coordinator(metadata.clone(), &provider);
    let now = crate::unix_now().unwrap();
    assert_eq!(first.dispatch_available(now).await.unwrap(), 1);
    assert_eq!(
        metadata.runs().run("run-0").await.unwrap().unwrap().state,
        RunState::Queued
    );
    assert!(
        metadata
            .runs()
            .claim_terminal_cloud_task_stops(now, 10)
            .await
            .unwrap()
            .is_empty()
    );
    drop(first);
    let restarted = coordinator(metadata.clone(), &provider);
    let mut clock = now;
    for expected in 2..=4 {
        let jobs = metadata.runs().run_jobs("run-0").await.unwrap();
        let due = jobs[0]
            .capacity_retry
            .as_ref()
            .unwrap()
            .next_attempt_at_unix
            .unwrap();
        assert!(due > clock);
        assert_eq!(restarted.dispatch_available(due - 1).await.unwrap(), 0);
        assert_eq!(restarted.dispatch_available(due).await.unwrap(), 1);
        assert_eq!(provider.count("start"), expected);
        clock = due;
    }
    assert_eq!(
        metadata.runs().run("run-0").await.unwrap().unwrap().state,
        RunState::Failed
    );
    assert_eq!(restarted.dispatch_available(now + 120).await.unwrap(), 0);
    assert_eq!(provider.count("start"), 4);
}

#[tokio::test]
async fn permanent_quota_and_uncertain_launches_do_not_enter_capacity_retries() {
    for (reply, expected) in [
        (
            json!({"status":"rejected","reason":"quota","message":"quota exceeded"}),
            RunState::Failed,
        ),
        (
            json!({"status":"rejected","reason":"permanent","message":"invalid configuration"}),
            RunState::Failed,
        ),
        (
            json!({"status":"ambiguous","message":"launch result unknown"}),
            RunState::Dispatching,
        ),
    ] {
        let metadata = queued_runs(1).await;
        let provider = FakeEcs::new().await;
        provider.reply(axum::http::StatusCode::OK, reply, false);
        let execution = coordinator(metadata.clone(), &provider);
        let now = crate::unix_now().unwrap();
        assert_eq!(execution.dispatch_available(now).await.unwrap(), 1);
        assert_eq!(
            metadata.runs().run("run-0").await.unwrap().unwrap().state,
            expected
        );
        assert_eq!(execution.dispatch_available(now + 119).await.unwrap(), 0);
        assert_eq!(provider.count("start"), 1);
    }
}

#[tokio::test]
async fn capacity_retry_can_start_and_expired_waits_settle_even_when_dispatch_is_paused() {
    for recover in [true, false] {
        let metadata = queued_runs(1).await;
        let provider = FakeEcs::new().await;
        provider.reply(
            axum::http::StatusCode::OK,
            json!({"status":"rejected","reason":"capacity","message":"capacity unavailable"}),
            false,
        );
        let mut execution = coordinator(metadata.clone(), &provider);
        let now = crate::unix_now().unwrap();
        execution.dispatch_available(now).await.unwrap();
        let jobs = metadata.runs().run_jobs("run-0").await.unwrap();
        let retry = jobs[0].capacity_retry.as_ref().unwrap();
        if recover {
            provider.reply(
                axum::http::StatusCode::OK,
                json!({"status":"started","task_arn":"task-recovered"}),
                false,
            );
            assert_eq!(
                execution
                    .dispatch_available(retry.next_attempt_at_unix.unwrap())
                    .await
                    .unwrap(),
                1
            );
            let detail = metadata.runs().run_detail("run-0").await.unwrap().unwrap();
            assert_eq!(detail.run.state, RunState::Dispatching);
            assert!(detail.attempts.iter().any(
                |attempt| attempt.attempt.external_run_id.as_deref() == Some("task-recovered")
            ));
        } else {
            execution.settings.max_concurrency = 0;
            assert_eq!(
                execution
                    .dispatch_available(retry.first_rejected_at_unix + 120)
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                metadata.runs().run("run-0").await.unwrap().unwrap().state,
                RunState::Failed
            );
            assert_eq!(provider.count("start"), 1);
        }
    }
}
