use super::*;
use crate::job_trace::{QueuedTrace, test_support::queued_by};

#[tokio::test]
async fn claimed_media_jobs_carry_the_trace_that_queued_them() {
    let fixture = fixture();
    start_request(&fixture, "traced_request", "traced-request", 1).await;
    queued_by(
        "trace-media-upload",
        upload_attachment(&fixture, "traced_request", "traced_attachment", 10),
    )
    .await;

    let (_, processing_trace) = fixture
        .store
        .media()
        .claim_processing_job("traced-processing", 20, 30)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        processing_trace,
        QueuedTrace::from_column(Some("trace-media-upload".into()))
    );

    queued_by(
        "trace-request-close",
        fixture.store.requests().close_request(CloseRequestCommand {
            request_id: "traced_request".to_string(),
            actor_user_id: OWNER_ID.to_string(),
            event_id: "event_close_traced_request".to_string(),
            now_unix: 22,
        }),
    )
    .await
    .unwrap();
    let (_, cleanup_trace) = fixture
        .store
        .media()
        .claim_cleanup_job("traced-cleanup", 90, 100)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        cleanup_trace,
        QueuedTrace::from_column(Some("trace-request-close".into()))
    );
}
