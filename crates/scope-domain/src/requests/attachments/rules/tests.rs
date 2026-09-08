use super::*;

fn prepared() -> RequestAttachment {
    validate_prepare_attachment(
        PrepareRequestAttachmentInput {
            attachment_id: "att_1".into(),
            repository_id: "repo_1".into(),
            request_id: "req_1".into(),
            uploader_user_id: "user_1".into(),
            upload_id: "upload_1".into(),
            operation_id: "operation_1".into(),
            target: RequestAttachmentTarget::Description,
            filename: "proof.png".into(),
            declared_media_type: "image/png".into(),
            size_bytes: 10,
            sha256: "a".repeat(64),
            actor_can_write_target: true,
            request_is_open: true,
            target_attachment_count: 0,
            request_source_bytes: 0,
            repository_reserved_bytes: 0,
            now_unix: 10,
        },
        RequestAttachmentLimits::default(),
    )
    .unwrap()
    .attachment
}

fn uploaded() -> RequestAttachment {
    finish_attachment_upload(
        &prepared(),
        "user_1",
        "upload_1",
        &[RequestAttachmentPartReceipt {
            part_number: 1,
            size_bytes: 10,
            sha256: "b".repeat(64),
        }],
        RequestAttachmentStoredObject {
            object_key: "originals/att_1/manifest".into(),
            size_bytes: 10,
            sha256: "a".repeat(64),
        },
        11,
        RequestAttachmentLimits::default(),
    )
    .unwrap()
}

fn processing_lease(attachment: &RequestAttachment) -> RequestAttachmentProcessingLease {
    RequestAttachmentProcessingLease {
        attachment_id: attachment.id.clone(),
        repository_id: attachment.repository_id.clone(),
        request_id: attachment.request_id.clone(),
        lease_token: "lease_1".into(),
        lease_generation: 2,
        attempt: 1,
        lease_expires_at_unix: 100,
    }
}

#[test]
fn prepare_enforces_media_and_aggregate_limits() {
    let mut input = PrepareRequestAttachmentInput {
        attachment_id: "att_1".into(),
        repository_id: "repo_1".into(),
        request_id: "req_1".into(),
        uploader_user_id: "user_1".into(),
        upload_id: "upload_1".into(),
        operation_id: "operation_1".into(),
        target: RequestAttachmentTarget::Description,
        filename: "huge.png".into(),
        declared_media_type: "image/png".into(),
        size_bytes: 26 * 1024 * 1024,
        sha256: "a".repeat(64),
        actor_can_write_target: true,
        request_is_open: true,
        target_attachment_count: 0,
        request_source_bytes: 0,
        repository_reserved_bytes: 0,
        now_unix: 10,
    };
    assert!(validate_prepare_attachment(input.clone(), Default::default()).is_err());
    input.size_bytes = 1;
    input.repository_reserved_bytes =
        RequestAttachmentLimits::default().max_repository_storage_bytes;
    assert!(validate_prepare_attachment(input, Default::default()).is_err());
}

#[test]
fn prepare_bounds_untrusted_metadata() {
    let mut input = PrepareRequestAttachmentInput {
        attachment_id: "att_1".into(),
        repository_id: "repo_1".into(),
        request_id: "req_1".into(),
        uploader_user_id: "user_1".into(),
        upload_id: "upload_1".into(),
        operation_id: "x".repeat(129),
        target: RequestAttachmentTarget::Description,
        filename: "proof.png".into(),
        declared_media_type: "image/png".into(),
        size_bytes: 10,
        sha256: "a".repeat(64),
        actor_can_write_target: true,
        request_is_open: true,
        target_attachment_count: 0,
        request_source_bytes: 0,
        repository_reserved_bytes: 0,
        now_unix: 10,
    };
    assert!(validate_prepare_attachment(input.clone(), Default::default()).is_err());
    input.operation_id = "operation_1".into();
    input.filename = "folder/proof.png".into();
    assert!(validate_prepare_attachment(input, Default::default()).is_err());
}

#[test]
fn finish_requires_exact_receipts_and_remains_idempotent_after_processing_starts() {
    let upload = uploaded();
    let processing =
        transition_attachment(&upload, RequestAttachmentState::Processing, None, 12).unwrap();
    let repeated = finish_attachment_upload(
        &processing,
        "user_1",
        "upload_1",
        &[RequestAttachmentPartReceipt {
            part_number: 1,
            size_bytes: 10,
            sha256: "b".repeat(64),
        }],
        processing.original.clone().unwrap(),
        13,
        Default::default(),
    )
    .unwrap();
    assert_eq!(repeated, processing);
}

#[test]
fn individual_part_must_match_its_exact_prepared_range() {
    let upload = prepared();
    let exact = RequestAttachmentPartReceipt {
        part_number: 1,
        size_bytes: 10,
        sha256: "b".repeat(64),
    };
    validate_attachment_part(&upload, &exact, Default::default()).unwrap();
    assert!(
        validate_attachment_part(
            &upload,
            &RequestAttachmentPartReceipt {
                part_number: 2,
                ..exact.clone()
            },
            Default::default(),
        )
        .is_err()
    );
    assert!(
        validate_attachment_part(
            &upload,
            &RequestAttachmentPartReceipt {
                size_bytes: 9,
                ..exact
            },
            Default::default(),
        )
        .is_err()
    );
}

#[test]
fn binding_comes_only_from_markdown_and_cannot_cross_targets() {
    let upload = uploaded();
    let target = RequestAttachmentBindingTarget::Description;
    let bindings = replace_attachment_bindings(
        "req_1",
        "user_1",
        true,
        target,
        "![proof](/request-attachments/att_1)",
        std::slice::from_ref(&upload),
        &[],
        Default::default(),
    )
    .unwrap();
    assert!(can_view_request_attachment(
        &upload, &bindings, None, true, false
    ));
    assert!(
        replace_attachment_bindings(
            "req_1",
            "user_1",
            true,
            RequestAttachmentBindingTarget::Discussion {
                discussion_id: "disc_1".into(),
            },
            "[proof](/request-attachments/att_1)",
            &[upload],
            &bindings,
            Default::default(),
        )
        .is_err()
    );
}

#[test]
fn source_validation_enforces_detected_kind_pixel_and_duration_limits() {
    let upload = uploaded();
    let lease = processing_lease(&upload);
    assert!(
        mark_processing_source_validated(
            &upload,
            &lease,
            "lease_1",
            2,
            "video/mp4".into(),
            None,
            Some(RequestAttachmentVideoMetadata {
                width: 10,
                height: 10,
                duration_millis: 1,
            }),
            20,
        )
        .is_err()
    );
    assert!(
        mark_processing_source_validated(
            &upload,
            &lease,
            "lease_1",
            2,
            "image/png".into(),
            Some(RequestAttachmentImageMetadata {
                width: 10_000,
                height: 10_000,
            }),
            None,
            20,
        )
        .is_err()
    );
    let mut video_upload = upload;
    video_upload.kind = RequestAttachmentKind::Video;
    video_upload.declared_media_type = "video/mp4".into();
    let video_lease = processing_lease(&video_upload);
    assert!(
        mark_processing_source_validated(
            &video_upload,
            &video_lease,
            "lease_1",
            2,
            "video/mp4".into(),
            None,
            Some(RequestAttachmentVideoMetadata {
                width: 1920,
                height: 1080,
                duration_millis: 600_001,
            }),
            20,
        )
        .is_err()
    );
}

#[test]
fn validated_photo_becomes_ready_with_an_immutable_preview() {
    let upload = uploaded();
    assert!(!upload.original_is_grantable());
    let lease = processing_lease(&upload);
    let ready = validate_processing_completion(
        &upload,
        &lease,
        "lease_1",
        2,
        "image/png".into(),
        Some(RequestAttachmentImageMetadata {
            width: 100,
            height: 80,
        }),
        None,
        vec![RequestAttachmentDerivative {
            id: "preview_1".into(),
            kind: RequestAttachmentDerivativeKind::ImagePreview,
            media_type: "image/webp".into(),
            object: RequestAttachmentStoredObject {
                object_key: "derivatives/att_1/preview_1".into(),
                size_bytes: 8,
                sha256: "c".repeat(64),
            },
            width: Some(100),
            height: Some(80),
            duration_millis: None,
        }],
        20,
    )
    .unwrap();

    assert_eq!(ready.state, RequestAttachmentState::Ready);
    assert!(ready.original_is_grantable());
    assert_eq!(ready.derivatives.len(), 1);
}

#[test]
fn stale_processing_lease_cannot_publish() {
    let upload = uploaded();
    let lease = processing_lease(&upload);
    assert!(
        validate_processing_completion(
            &upload,
            &lease,
            "stale",
            2,
            "image/png".into(),
            Some(RequestAttachmentImageMetadata {
                width: 10,
                height: 10,
            }),
            None,
            vec![],
            20,
        )
        .is_err()
    );
}

#[test]
fn rejected_is_terminal_but_retryable_failure_can_restart() {
    let upload = uploaded();
    let failed = transition_attachment(
        &upload,
        RequestAttachmentState::Failed,
        Some(RequestAttachmentFailure {
            code: RequestAttachmentFailureCode::StorageUnavailable,
            message: "temporary".into(),
            retryable: true,
        }),
        12,
    )
    .unwrap();
    assert_eq!(
        retry_attachment_processing(&failed, true, 13)
            .unwrap()
            .state,
        RequestAttachmentState::Processing
    );
    let rejected = transition_attachment(
        &upload,
        RequestAttachmentState::Rejected,
        Some(RequestAttachmentFailure {
            code: RequestAttachmentFailureCode::CorruptMedia,
            message: "corrupt".into(),
            retryable: false,
        }),
        12,
    )
    .unwrap();
    assert!(retry_attachment_processing(&rejected, true, 13).is_err());
}
