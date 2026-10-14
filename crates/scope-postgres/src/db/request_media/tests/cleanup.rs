use super::*;
use scope_domain::requests::attachments::{RequestAttachmentCleanupLease, RequestAttachmentLimits};

async fn claim_cleanup(
    fixture: &Fixture,
    token: &str,
    now_unix: u64,
    expires_at_unix: u64,
) -> RequestAttachmentCleanupLease {
    fixture
        .store
        .media()
        .claim_cleanup_job(token, now_unix, expires_at_unix)
        .await
        .unwrap()
        .unwrap()
}

async fn abandon_first_part_write(fixture: &Fixture, attachment_id: &str) {
    let prepared = prepare_attachment(fixture, "orphan_request", attachment_id, 10).await;
    for (suffix, now_unix) in [("first", 11), ("replacement", 12)] {
        let staged = part(attachment_id, suffix);
        assert_eq!(
            fixture
                .store
                .media()
                .reserve_upload_part(
                    attachment_id,
                    &prepared.upload_id,
                    OWNER_ID,
                    staged.clone(),
                    &format!("write_{suffix}"),
                    now_unix,
                    now_unix + 1,
                )
                .await
                .unwrap(),
            ReserveUploadPartResult::Write(staged)
        );
    }
}

#[tokio::test]
async fn orphan_claims_skip_live_leases_and_back_off() {
    let fixture = fixture();
    start_request(&fixture, "orphan_request", "orphan-request", 1).await;
    for attachment_id in ["orphan_a", "orphan_b", "orphan_c", "orphan_d"] {
        abandon_first_part_write(&fixture, attachment_id).await;
    }
    let media = fixture.store.media();

    let first = claim_cleanup(&fixture, "first", 20, 40).await;
    assert_eq!(first.attachment_id, "orphan_a");
    assert_eq!(
        first.object_keys,
        ["media/v1/staged/orphan_a/first".to_string()]
    );
    let second = claim_cleanup(&fixture, "second", 20, 40).await;
    assert_eq!(second.attachment_id, "orphan_b");
    assert_eq!(
        media
            .fail_cleanup_job(
                &first.attachment_id,
                &first.lease_token,
                first.lease_generation,
                21,
                100,
                "retry",
            )
            .await
            .unwrap(),
        MediaLeaseMutation::Applied(())
    );

    assert_eq!(
        claim_cleanup(&fixture, "third", 22, 42).await.attachment_id,
        "orphan_c"
    );
    assert_eq!(
        claim_cleanup(&fixture, "fourth", 22, 42)
            .await
            .attachment_id,
        "orphan_d"
    );
    assert!(
        media
            .claim_cleanup_job("none", 23, 43)
            .await
            .unwrap()
            .is_none()
    );

    let retried = claim_cleanup(&fixture, "after-backoff", 100, 120).await;
    assert_eq!(retried.attachment_id, "orphan_a");
    assert_eq!(retried.lease_generation, first.lease_generation + 1);
    assert_eq!(retried.attempt, 2);
}

#[tokio::test]
async fn completed_cleanup_never_reserves_bytes_again_during_reconciliation() {
    let fixture = fixture();
    start_request(&fixture, "budget_request", "budget-request", 1).await;
    let old = prepare_attachment(&fixture, "budget_request", "old_attachment", 10).await;
    let expired = old.attachment.upload_expires_at_unix;
    let media = fixture.store.media();
    media
        .enqueue_expired_attachment_cleanup(expired)
        .await
        .unwrap();
    let lease = claim_cleanup(&fixture, "delete", expired, expired + 10).await;
    assert_eq!(
        media
            .complete_cleanup_job(
                &lease.attachment_id,
                &lease.lease_token,
                lease.lease_generation,
                expired + 1,
            )
            .await
            .unwrap(),
        MediaLeaseMutation::Applied(())
    );

    let sweep = expired + 1 + 24 * 60 * 60;
    let current =
        prepare_attachment(&fixture, "budget_request", "current_attachment", sweep - 1).await;
    media
        .enqueue_expired_attachment_cleanup(sweep)
        .await
        .unwrap();
    for leased in [false, true] {
        if leased {
            let retry = claim_cleanup(&fixture, "sweep", sweep, sweep + 10).await;
            assert_eq!(retry.attachment_id, "old_attachment");
        }
        let usage = super::super::budget::media_usage(
            fixture.store.db.as_ref(),
            &fixture.repository_id,
            Some("budget_request"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(usage.request_source_bytes, 4);
        assert_eq!(usage.repository_bytes, 8);
        super::super::processing_support::ensure_derivative_budget(
            fixture.store.db.as_ref(),
            &current.attachment,
            RequestAttachmentLimits::default().max_repository_storage_bytes
                - current.attachment.size_bytes,
        )
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_orphan_claimers_each_take_distinct_work() {
    let fixture = fixture();
    start_request(&fixture, "orphan_request", "orphan-request", 1).await;
    let attachment_ids: Vec<String> = (0..10).map(|index| format!("orphan_{index}")).collect();
    for attachment_id in &attachment_ids {
        abandon_first_part_write(&fixture, attachment_id).await;
    }
    let mut claims = tokio::task::JoinSet::new();
    for claimer in 0..attachment_ids.len() {
        let media = fixture.store.media();
        claims.spawn(async move {
            media
                .claim_cleanup_job(&format!("claimer_{claimer}"), 20, 40)
                .await
        });
    }
    let mut claimed = Vec::new();
    while let Some(claim) = claims.join_next().await {
        claimed.push(claim.unwrap().unwrap().unwrap().attachment_id);
    }
    claimed.sort();
    assert_eq!(claimed, attachment_ids);
}
