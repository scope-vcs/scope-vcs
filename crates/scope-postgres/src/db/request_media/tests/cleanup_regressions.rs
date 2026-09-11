use super::*;

#[tokio::test]
async fn orphan_claims_skip_live_leases_backoff_and_concurrent_claimers() {
    let fixture = fixture();
    start_request(&fixture, "orphan_request", "orphan-request", 1).await;
    for id in ["orphan_a", "orphan_b", "orphan_c", "orphan_d"] {
        let prepared = prepare_attachment(&fixture, "orphan_request", id, 10).await;
        for (suffix, now) in [("first", 11), ("replacement", 12)] {
            assert!(matches!(
                fixture
                    .store
                    .media()
                    .reserve_upload_part(
                        id,
                        &prepared.upload_id,
                        OWNER_ID,
                        part(id, suffix),
                        suffix,
                        now,
                        now + 1,
                    )
                    .await
                    .unwrap(),
                ReserveUploadPartResult::Write(_)
            ));
        }
    }
    let media = fixture.store.media();
    let first = media
        .claim_cleanup_job("first", 20, 40)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.attachment_id, "orphan_a");
    let second = media
        .claim_cleanup_job("second", 20, 40)
        .await
        .unwrap()
        .unwrap();
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
    let (third, fourth) = tokio::join!(
        media.claim_cleanup_job("third", 22, 42),
        media.claim_cleanup_job("fourth", 22, 42),
    );
    let mut ids = [
        third.unwrap().unwrap().attachment_id,
        fourth.unwrap().unwrap().attachment_id,
    ];
    ids.sort();
    assert_eq!(ids, ["orphan_c", "orphan_d"]);
    assert!(
        media
            .claim_cleanup_job("none", 23, 43)
            .await
            .unwrap()
            .is_none()
    );
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
    let lease = media
        .claim_cleanup_job("delete", expired, expired + 10)
        .await
        .unwrap()
        .unwrap();
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
    let before = super::super::budget::media_usage(
        fixture.store.db.as_ref(),
        &fixture.repository_id,
        Some("budget_request"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(before.request_source_bytes, 4);
    media
        .enqueue_expired_attachment_cleanup(sweep)
        .await
        .unwrap();
    for leased in [false, true] {
        if leased {
            let retry = media
                .claim_cleanup_job("sweep", sweep, sweep + 10)
                .await
                .unwrap()
                .unwrap();
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
        assert_eq!(usage.request_source_bytes, before.request_source_bytes);
        assert_eq!(usage.repository_bytes, before.repository_bytes);
        super::super::processing_support::ensure_derivative_budget(
            fixture.store.db.as_ref(),
            &current.attachment,
            default_limits().max_repository_storage_bytes - current.attachment.size_bytes,
        )
        .await
        .unwrap();
    }
    let mut limits = default_limits();
    limits.max_request_source_bytes = 8;
    media
        .prepare_request_attachment(
            PrepareRequestAttachmentCommand {
                attachment_id: "next_attachment".into(),
                upload_id: "next_upload".into(),
                operation_id: "next_operation".into(),
                request_id: "budget_request".into(),
                actor_user_id: OWNER_ID.into(),
                target: RequestAttachmentTarget::Description,
                filename: "next.png".into(),
                declared_media_type: "image/png".into(),
                size_bytes: 4,
                sha256: SOURCE_SHA256.into(),
                now_unix: sweep + 1,
            },
            limits,
        )
        .await
        .unwrap();
}
