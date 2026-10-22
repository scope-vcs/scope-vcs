use super::*;

async fn bind_and_submit(fixture: &Fixture, request_id: &str, attachment_id: &str) {
    fixture
        .store
        .requests()
        .edit_request_identity(EditRequestIdentityCommand {
            request_id: request_id.to_string(),
            actor_user_id: OWNER_ID.to_string(),
            event_id: format!("event_bind_{attachment_id}"),
            title: None,
            description_markdown: Some(attachment_markdown(attachment_id)),
            expected_description_markdown: Some(String::new()),
            now_unix: 30,
        })
        .await
        .unwrap();
    fixture
        .store
        .requests()
        .mutate_request_for_tests(request_id, |request| {
            request.submitted_at_unix = Some(31);
            request.updated_at_unix = 31;
        })
        .await
        .unwrap();
}

async fn visible_ids(fixture: &Fixture, request_id: &str, viewer: Option<&str>) -> Vec<String> {
    fixture
        .store
        .media()
        .list_request_attachments_for_viewer(request_id, viewer)
        .await
        .unwrap()
        .into_iter()
        .map(|attachment| attachment.id)
        .collect()
}

#[tokio::test]
async fn attachment_lists_show_unbound_drafts_only_to_their_uploader() {
    let fixture = fixture();
    start_request(&fixture, "list_request", "list-request", 1).await;
    upload_attachment(&fixture, "list_request", "bound_attachment", 10).await;
    upload_attachment(&fixture, "list_request", "draft_attachment", 20).await;
    fixture
        .store
        .requests()
        .edit_request_identity(EditRequestIdentityCommand {
            request_id: "list_request".to_string(),
            actor_user_id: OWNER_ID.to_string(),
            event_id: "event_bind_before_submit".to_string(),
            title: None,
            description_markdown: Some(attachment_markdown("bound_attachment")),
            expected_description_markdown: Some(String::new()),
            now_unix: 30,
        })
        .await
        .unwrap();
    assert!(
        visible_ids(&fixture, "list_request", Some(VIEWER_ID))
            .await
            .is_empty()
    );

    fixture
        .store
        .requests()
        .mutate_request_for_tests("list_request", |request| {
            request.submitted_at_unix = Some(31);
            request.updated_at_unix = 31;
        })
        .await
        .unwrap();
    assert_eq!(
        visible_ids(&fixture, "list_request", Some(OWNER_ID)).await,
        ["bound_attachment", "draft_attachment"]
    );
    for viewer in [Some(VIEWER_ID), None] {
        assert_eq!(
            visible_ids(&fixture, "list_request", viewer).await,
            ["bound_attachment"]
        );
    }
}

#[tokio::test]
async fn single_attachment_and_media_reads_follow_list_visibility() {
    let fixture = fixture();
    start_request(&fixture, "read_request", "read-request", 1).await;
    upload_attachment(&fixture, "read_request", "bound_attachment", 10).await;
    upload_attachment(&fixture, "read_request", "draft_attachment", 20).await;
    bind_and_submit(&fixture, "read_request", "bound_attachment").await;
    let media = fixture.store.media();

    for (viewer, attachment_id, visible) in [
        (Some(OWNER_ID), "draft_attachment", true),
        (Some(VIEWER_ID), "bound_attachment", true),
        (None, "bound_attachment", true),
        (Some(VIEWER_ID), "draft_attachment", false),
        (None, "draft_attachment", false),
    ] {
        let read = media
            .request_attachment_for_viewer("read_request", attachment_id, viewer)
            .await
            .unwrap();
        assert_eq!(read.is_some(), visible, "{viewer:?} {attachment_id}");
        let media_read = media
            .request_attachment_media_for_viewer(
                "read_request",
                attachment_id,
                viewer,
                RequestMediaObjectTarget::Original,
            )
            .await
            .unwrap();
        assert_eq!(media_read.is_some(), visible, "{viewer:?} {attachment_id}");
    }
    assert!(
        media
            .request_attachment_for_viewer("other_request", "bound_attachment", Some(OWNER_ID))
            .await
            .unwrap()
            .is_none()
    );

    let (_, unvalidated) = media
        .request_attachment_media_for_viewer(
            "read_request",
            "bound_attachment",
            Some(VIEWER_ID),
            RequestMediaObjectTarget::Original,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(unvalidated.is_none());

    let (lease, _) = media
        .claim_processing_job(
            RequestAttachmentProcessingLane::General,
            "access-processing-token",
            40,
            60,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.attachment_id, "bound_attachment");
    let validated = media
        .mark_processing_source_validated(ValidateRequestAttachmentSourceCommand {
            attachment_id: lease.attachment_id,
            lease_token: lease.lease_token,
            lease_generation: lease.lease_generation,
            source: png_source(),
            now_unix: 41,
        })
        .await
        .unwrap();
    assert!(matches!(validated, MediaLeaseMutation::Applied(_)));

    let (attachment, original) = media
        .request_attachment_media_for_viewer(
            "read_request",
            "bound_attachment",
            Some(VIEWER_ID),
            RequestMediaObjectTarget::Original,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(attachment.id, "bound_attachment");
    let original = original.unwrap();
    assert_eq!(original.media_type, "image/png");
    assert_eq!(original.plaintext_bytes, 4);
    let (_, derivative) = media
        .request_attachment_media_for_viewer(
            "read_request",
            "bound_attachment",
            Some(VIEWER_ID),
            RequestMediaObjectTarget::Derivative("missing"),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(derivative.is_none());
}

#[tokio::test]
async fn retry_replay_returns_the_saved_attachment_only_to_viewers_who_can_see_it() {
    let fixture = fixture();
    start_request(&fixture, "retry_request", "retry-request", 1).await;
    upload_attachment(&fixture, "retry_request", "bound_attachment", 10).await;
    upload_attachment(&fixture, "retry_request", "retry_attachment", 20).await;
    bind_and_submit(&fixture, "retry_request", "bound_attachment").await;
    let media = fixture.store.media();

    let retried = media
        .retry_request_attachment_processing(
            "retry_request",
            "retry_attachment",
            OWNER_ID,
            "retry_operation",
            32,
        )
        .await
        .unwrap();
    assert_eq!(retried.state, RequestAttachmentState::Processing);
    let denied = media
        .retry_request_attachment_processing(
            "retry_request",
            "retry_attachment",
            VIEWER_ID,
            "retry_operation",
            33,
        )
        .await
        .unwrap_err();
    assert_eq!(denied.kind, PostgresErrorKind::NotFound);

    fixture
        .store
        .requests()
        .close_request(CloseRequestCommand {
            request_id: "retry_request".to_string(),
            actor_user_id: OWNER_ID.to_string(),
            event_id: "event_close_retry".to_string(),
            now_unix: 34,
        })
        .await
        .unwrap();
    let replayed = media
        .retry_request_attachment_processing(
            "retry_request",
            "retry_attachment",
            OWNER_ID,
            "retry_operation",
            35,
        )
        .await
        .unwrap();
    assert_eq!(replayed.id, "retry_attachment");
    assert_eq!(replayed.state, RequestAttachmentState::Processing);
}

#[tokio::test]
async fn media_reads_load_derivative_manifests_until_cleanup_tombstones_the_attachment() {
    let fixture = fixture();
    start_request(&fixture, "derivative_request", "derivative-request", 1).await;
    upload_attachment(&fixture, "derivative_request", "derivative_attachment", 10).await;
    let media = fixture.store.media();
    let (lease, _) = media
        .claim_processing_job(
            RequestAttachmentProcessingLane::General,
            "derivative-token",
            20,
            40,
        )
        .await
        .unwrap()
        .unwrap();
    let output_key = "media/v1/staged/derivative_attachment/preview";
    media
        .reserve_processing_object_key(
            "derivative_attachment",
            &lease.lease_token,
            lease.lease_generation,
            output_key,
            21,
        )
        .await
        .unwrap();
    let chunk = RequestMediaChunk {
        part_number: 1,
        plaintext_offset: 0,
        plaintext_bytes: 3,
        sha256: "b".repeat(64),
        object_key: output_key.to_string(),
    };
    let completed = media
        .complete_processing_job(CompleteRequestAttachmentProcessingCommand {
            attachment_id: "derivative_attachment".to_string(),
            lease_token: lease.lease_token,
            lease_generation: lease.lease_generation,
            source: png_source(),
            derivatives: vec![CompletedRequestAttachmentDerivative {
                derivative: RequestAttachmentDerivative {
                    id: "derivative_preview".to_string(),
                    kind: RequestAttachmentDerivativeKind::ImagePreview,
                    media_type: "image/webp".to_string(),
                    object: RequestAttachmentStoredObject {
                        object_key: "manifest_derivative_preview".to_string(),
                        size_bytes: 3,
                        sha256: "b".repeat(64),
                    },
                    width: Some(1),
                    height: Some(1),
                    duration_millis: None,
                },
                chunks: vec![chunk.clone()],
            }],
            now_unix: 22,
        })
        .await
        .unwrap();
    let MediaLeaseMutation::Applied(completed) = completed else {
        panic!("current processing lease should complete");
    };
    assert_eq!(completed.state, RequestAttachmentState::Ready);
    assert_eq!(completed.derivatives.len(), 1);
    let inventory = fixture
        .store
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT state, manifest_id FROM scope_request_media_processing_objects
             WHERE object_key = $1",
            [output_key.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(inventory.try_get::<String>("", "state").unwrap(), "Adopted");
    assert_eq!(
        inventory.try_get::<String>("", "manifest_id").unwrap(),
        "manifest_derivative_preview"
    );

    let read = |target| {
        media.request_attachment_media_for_viewer(
            "derivative_request",
            "derivative_attachment",
            Some(OWNER_ID),
            target,
        )
    };
    let (attachment, derivative) = read(RequestMediaObjectTarget::Derivative("derivative_preview"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(attachment.derivatives.len(), 1);
    let derivative = derivative.unwrap();
    assert_eq!(derivative.media_type, "image/webp");
    assert_eq!(derivative.chunks, [chunk]);

    fixture
        .store
        .db
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_request_media_cleanup_jobs (
                attachment_id, repository_id, reason, state, available_at_unix,
                created_at_unix, updated_at_unix
             ) VALUES ($1, $2, 'RequestDeleted', 'Queued', 30, 30, 30)",
            [
                "derivative_attachment".into(),
                fixture.repository_id.clone().into(),
            ],
        ))
        .await
        .unwrap();
    for target in [
        RequestMediaObjectTarget::Original,
        RequestMediaObjectTarget::Derivative("derivative_preview"),
    ] {
        assert!(read(target).await.unwrap().is_none(), "{target:?}");
    }
    assert!(
        media
            .request_attachment_for_viewer(
                "derivative_request",
                "derivative_attachment",
                Some(OWNER_ID)
            )
            .await
            .unwrap()
            .is_none()
    );
}
