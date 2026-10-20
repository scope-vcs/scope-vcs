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
        .0
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
            .complete_cleanup_job(&lease, expired + 1)
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
        claimed.push(claim.unwrap().unwrap().unwrap().0.attachment_id);
    }
    claimed.sort();
    assert_eq!(claimed, attachment_ids);
}

const DAY: u64 = 24 * 60 * 60;

async fn upload_after_abandoning_a_part(fixture: &Fixture, attachment_id: &str) -> String {
    abandon_first_part_write(fixture, attachment_id).await;
    let media = fixture.store.media();
    let replacement = part(attachment_id, "replacement");
    assert_eq!(
        media
            .mark_upload_part_stored(
                attachment_id,
                &format!("upload_{attachment_id}"),
                OWNER_ID,
                1,
                &replacement.object_key,
                "write_replacement",
                12,
            )
            .await
            .unwrap(),
        StorePartResult::Recorded
    );
    media
        .finish_request_attachment_upload(FinishRequestAttachmentUploadCommand {
            request_id: "orphan_request".to_string(),
            attachment_id: attachment_id.to_string(),
            upload_id: format!("upload_{attachment_id}"),
            actor_user_id: OWNER_ID.to_string(),
            parts: vec![replacement.receipt],
            now_unix: 12,
        })
        .await
        .unwrap();
    format!("media/v1/staged/{attachment_id}/first")
}

async fn orphan_processing_output(fixture: &Fixture, attachment_id: &str, now_unix: u64) -> String {
    let media = fixture.store.media();
    let (lease, _) = media
        .claim_processing_job(
            RequestAttachmentProcessingLane::General,
            &format!("output-{now_unix}"),
            now_unix,
            now_unix + 10,
        )
        .await
        .unwrap()
        .unwrap();
    let output = format!("media/v1/staged/{attachment_id}/output-{now_unix}");
    assert_eq!(
        media
            .reserve_processing_object_key(
                attachment_id,
                &lease.lease_token,
                lease.lease_generation,
                &output,
                now_unix + 1,
            )
            .await
            .unwrap(),
        MediaLeaseMutation::Applied(())
    );
    media
        .claim_processing_job(
            RequestAttachmentProcessingLane::General,
            &format!("takeover-{now_unix}"),
            now_unix + 10,
            now_unix + 20,
        )
        .await
        .unwrap()
        .unwrap();
    output
}

#[tokio::test]
async fn completed_orphan_inventory_is_reconciled_once_then_never() {
    let fixture = fixture();
    start_request(&fixture, "orphan_request", "orphan-request", 1).await;
    let attachment_id = "reconciled_attachment";
    let abandoned_part = upload_after_abandoning_a_part(&fixture, attachment_id).await;
    let orphaned_output = orphan_processing_output(&fixture, attachment_id, 20).await;
    let expected_keys = [abandoned_part, orphaned_output];
    let media = fixture.store.media();

    let mut sweep = 200;
    for _ in 0..2 {
        media
            .enqueue_expired_attachment_cleanup(sweep)
            .await
            .unwrap();
        let lease = claim_cleanup(&fixture, "orphans", sweep, sweep + 10).await;
        assert_eq!(lease.object_keys, expected_keys);
        let completed_at = sweep + 1;
        assert_eq!(
            media
                .complete_cleanup_job(&lease, completed_at)
                .await
                .unwrap(),
            MediaLeaseMutation::Applied(())
        );
        let too_early = completed_at + DAY - 1;
        media
            .enqueue_expired_attachment_cleanup(too_early)
            .await
            .unwrap();
        assert!(
            media
                .claim_cleanup_job("too-early", too_early, too_early + 10)
                .await
                .unwrap()
                .is_none()
        );
        sweep = completed_at + DAY;
    }
    media
        .enqueue_expired_attachment_cleanup(sweep)
        .await
        .unwrap();
    assert!(
        media
            .claim_cleanup_job("never-again", sweep, sweep + 10)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn orphan_completion_deletes_only_its_leased_keys_when_a_sweep_interleaves() {
    let fixture = fixture();
    start_request(&fixture, "orphan_request", "orphan-request", 1).await;
    let attachment_id = "interleaved_attachment";
    let abandoned_part = upload_after_abandoning_a_part(&fixture, attachment_id).await;
    let orphaned_output = orphan_processing_output(&fixture, attachment_id, 20).await;
    let media = fixture.store.media();
    let first = claim_cleanup(&fixture, "first-orphans", 200, 210).await;
    assert_eq!(
        media.complete_cleanup_job(&first, 201).await.unwrap(),
        MediaLeaseMutation::Applied(())
    );

    let sweep = 201 + DAY;
    let late_output = orphan_processing_output(&fixture, attachment_id, sweep - 20).await;
    let leased = claim_cleanup(&fixture, "late-orphans", sweep, sweep + 10).await;
    assert_eq!(leased.object_keys, [late_output]);
    media
        .enqueue_expired_attachment_cleanup(sweep)
        .await
        .unwrap();
    assert_eq!(
        media
            .complete_cleanup_job(&leased, sweep + 1)
            .await
            .unwrap(),
        MediaLeaseMutation::Applied(())
    );

    let reconciled = claim_cleanup(&fixture, "reconciled-orphans", sweep + 2, sweep + 12).await;
    assert_eq!(reconciled.object_keys, [abandoned_part, orphaned_output]);
}
