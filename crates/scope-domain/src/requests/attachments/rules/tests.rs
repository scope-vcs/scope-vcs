use super::*;

fn prepared() -> RequestAttachment {
    validate_prepare_attachment(PrepareRequestAttachmentInput {
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
    })
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
    )
    .unwrap()
}

fn source(
    detected_media_type: &str,
    image: Option<RequestAttachmentImageMetadata>,
    video: Option<RequestAttachmentVideoMetadata>,
) -> RequestAttachmentSource {
    RequestAttachmentSource {
        detected_media_type: detected_media_type.into(),
        size_bytes: 10,
        sha256: "a".repeat(64),
        image,
        video,
    }
}

fn photo_preview() -> Vec<RequestAttachmentDerivative> {
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
    }]
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
    assert!(validate_prepare_attachment(input.clone()).is_err());
    input.size_bytes = 1;
    input.repository_reserved_bytes =
        RequestAttachmentLimits::default().max_repository_storage_bytes;
    assert!(validate_prepare_attachment(input).is_err());
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
    assert!(validate_prepare_attachment(input.clone()).is_err());
    input.operation_id = "operation_1".into();
    input.filename = "folder/proof.png".into();
    assert!(validate_prepare_attachment(input).is_err());
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
    validate_attachment_part(&upload, &exact).unwrap();
    assert!(
        validate_attachment_part(
            &upload,
            &RequestAttachmentPartReceipt {
                part_number: 2,
                ..exact.clone()
            },
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
        )
        .is_err()
    );
}

#[test]
fn source_validation_enforces_identity_detected_kind_pixel_and_duration_limits() {
    let upload = uploaded();
    let video = |duration_millis| {
        Some(RequestAttachmentVideoMetadata {
            width: 1920,
            height: 1080,
            duration_millis,
        })
    };
    let photo = Some(RequestAttachmentImageMetadata {
        width: 100,
        height: 80,
    });
    assert!(
        mark_processing_source_validated(&upload, source("video/mp4", None, video(1)), 20).is_err()
    );
    assert!(
        mark_processing_source_validated(
            &upload,
            source(
                "image/png",
                Some(RequestAttachmentImageMetadata {
                    width: 10_000,
                    height: 10_000,
                }),
                None,
            ),
            20,
        )
        .is_err()
    );
    let mut mismatched = source("image/png", photo, None);
    mismatched.size_bytes = 11;
    assert_eq!(
        mark_processing_source_validated(&upload, mismatched, 20).unwrap_err(),
        DomainError::conflict("validated source does not match prepared attachment")
    );
    let mut video_upload = upload;
    video_upload.kind = RequestAttachmentKind::Video;
    video_upload.declared_media_type = "video/mp4".into();
    assert!(
        mark_processing_source_validated(
            &video_upload,
            source("video/mp4", None, video(600_001)),
            20
        )
        .is_err()
    );
}

#[test]
fn validated_photo_becomes_ready_with_an_immutable_preview() {
    let upload = uploaded();
    assert!(!upload.original_is_grantable());
    let ready = validate_processing_completion(
        &upload,
        source(
            "image/png",
            Some(RequestAttachmentImageMetadata {
                width: 100,
                height: 80,
            }),
            None,
        ),
        photo_preview(),
        20,
    )
    .unwrap();

    assert_eq!(ready.state, RequestAttachmentState::Ready);
    assert!(ready.original_is_grantable());
    assert_eq!(ready.derivatives.len(), 1);
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
