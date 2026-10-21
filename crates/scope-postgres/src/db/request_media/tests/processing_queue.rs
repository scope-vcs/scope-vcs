use super::*;
use scope_domain::requests::attachments::{
    REQUEST_ATTACHMENT_MAX_PROCESSING_ATTEMPTS, RequestAttachmentFailureCode,
    RequestAttachmentKind, RequestAttachmentProcessingLease,
};

async fn claim(
    fixture: &Fixture,
    lane: RequestAttachmentProcessingLane,
    token: &str,
    now_unix: u64,
) -> Option<RequestAttachmentProcessingLease> {
    fixture
        .store
        .media()
        .claim_processing_job(lane, token, now_unix, now_unix + 10)
        .await
        .unwrap()
        .map(|(lease, _)| lease)
}

async fn column(fixture: &Fixture, sql: &str, key: &str) -> String {
    fixture
        .store
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            [key.into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "state")
        .unwrap()
}

#[tokio::test]
async fn photo_lane_skips_videos_and_general_lane_takes_photos_first() {
    let fixture = fixture();
    start_request(&fixture, "lane_request", "lane-request", 1).await;
    upload_attachment_as(&fixture, "lane_request", "older_video", "video/mp4", 10).await;
    upload_attachment(&fixture, "lane_request", "first_photo", 20).await;
    upload_attachment(&fixture, "lane_request", "second_photo", 30).await;
    let photo = RequestAttachmentProcessingLane::Photo;
    let general = RequestAttachmentProcessingLane::General;

    let photo_claim = claim(&fixture, photo, "photo-lane", 40).await.unwrap();
    assert_eq!(photo_claim.attachment_id, "first_photo");
    assert_eq!(photo_claim.kind, RequestAttachmentKind::Photo);
    let general_claim = claim(&fixture, general, "general-lane", 40).await.unwrap();
    assert_eq!(general_claim.attachment_id, "second_photo");
    assert!(
        claim(&fixture, photo, "idle-photo-lane", 40)
            .await
            .is_none()
    );
    let video_claim = claim(&fixture, general, "general-video", 40).await.unwrap();
    assert_eq!(video_claim.attachment_id, "older_video");
    assert_eq!(video_claim.kind, RequestAttachmentKind::Video);
}

#[tokio::test]
async fn an_expired_final_attempt_fails_the_attachment_until_a_manual_retry() {
    let fixture = fixture();
    start_request(&fixture, "capped_request", "capped-request", 1).await;
    upload_attachment(&fixture, "capped_request", "capped_attachment", 10).await;
    let general = RequestAttachmentProcessingLane::General;
    let mut final_lease = None;
    for attempt in 1..=REQUEST_ATTACHMENT_MAX_PROCESSING_ATTEMPTS {
        let lease = claim(
            &fixture,
            general,
            &format!("attempt-{attempt}"),
            20 * u64::from(attempt),
        )
        .await
        .unwrap();
        assert_eq!(lease.attempt, attempt);
        final_lease = Some(lease);
    }
    let final_lease = final_lease.unwrap();
    let interrupted_output = "media/v1/staged/capped_attachment/interrupted-output";
    let reserved_at = 20 * u64::from(REQUEST_ATTACHMENT_MAX_PROCESSING_ATTEMPTS) + 1;
    assert_eq!(
        fixture
            .store
            .media()
            .reserve_processing_object_key(
                "capped_attachment",
                &final_lease.lease_token,
                final_lease.lease_generation,
                interrupted_output,
                reserved_at,
            )
            .await
            .unwrap(),
        MediaLeaseMutation::Applied(())
    );

    let after_expiry = reserved_at + 20;
    assert!(
        claim(&fixture, general, "after-final-attempt", after_expiry)
            .await
            .is_none()
    );
    let failed =
        super::super::persistence::attachment_by_id(fixture.store.db.as_ref(), "capped_attachment")
            .await
            .unwrap()
            .unwrap();
    assert_eq!(failed.state, RequestAttachmentState::Failed);
    let failure = failed.failure.unwrap();
    assert_eq!(failure.code, RequestAttachmentFailureCode::Internal);
    assert!(failure.retryable);
    assert_eq!(
        column(
            &fixture,
            "SELECT state FROM scope_request_media_processing_jobs WHERE attachment_id = $1",
            "capped_attachment",
        )
        .await,
        "Completed"
    );
    assert_eq!(
        column(
            &fixture,
            "SELECT state FROM scope_request_media_processing_objects WHERE object_key = $1",
            interrupted_output,
        )
        .await,
        "Orphaned"
    );

    fixture
        .store
        .media()
        .retry_request_attachment_processing(
            "capped_request",
            "capped_attachment",
            OWNER_ID,
            "manual-retry",
            after_expiry + 1,
        )
        .await
        .unwrap();
    let retried = claim(&fixture, general, "manual-retry-claim", after_expiry + 1)
        .await
        .unwrap();
    assert_eq!(retried.attempt, 1);
}
