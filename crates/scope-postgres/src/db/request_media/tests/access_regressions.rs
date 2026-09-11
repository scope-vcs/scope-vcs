use super::*;
use sea_orm::{ActiveModelTrait, IntoActiveModel};

async fn add_viewer(fixture: &Fixture) {
    crate::db::entities::user::Model::from_domain(&UserAccount {
        id: "media_viewer".into(),
        handle: "media-viewer".into(),
        email: "viewer@example.com".into(),
        email_verified: true,
    })
    .into_active_model()
    .insert(fixture.store.db.as_ref())
    .await
    .unwrap();
}

#[tokio::test]
async fn attachment_lists_preserve_visibility_without_loading_manifest_chunks() {
    let fixture = fixture();
    add_viewer(&fixture).await;
    start_request(&fixture, "list_request", "list-request", 1).await;
    upload_attachment(&fixture, "list_request", "bound_attachment", 10).await;
    upload_attachment(&fixture, "list_request", "unbound_attachment", 20).await;
    fixture
        .store
        .requests()
        .edit_request_identity(EditRequestIdentityCommand {
            request_id: "list_request".into(),
            actor_user_id: OWNER_ID.into(),
            event_id: "bind".into(),
            title: None,
            description_markdown: Some(attachment_markdown("bound_attachment")),
            expected_description_markdown: Some(String::new()),
            now_unix: 30,
        })
        .await
        .unwrap();
    let media = fixture.store.media();
    assert!(
        media
            .list_request_attachments_for_viewer("list_request", Some("media_viewer"))
            .await
            .unwrap()
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
    // Retained expired inventory must not enter the list or its metadata batches.
    prepare_attachment(&fixture, "list_request", "deleted_attachment", 32).await;
    let tx = fixture.store.db.begin().await.unwrap();
    tx.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_request_media_cleanup_jobs
         (attachment_id, repository_id, reason, state, available_at_unix, created_at_unix, updated_at_unix)
         VALUES ($1, $2, 'IncompleteUploadExpired', 'Queued', 40, 40, 40)",
        ["deleted_attachment".into(), fixture.repository_id.clone().into()],
    )).await.unwrap();
    tx.commit().await.unwrap();
    let held = fixture.store.db.begin().await.unwrap();
    held.execute_unprepared(
        "LOCK TABLE scope_request_media_manifest_chunks IN ACCESS EXCLUSIVE MODE",
    )
    .await
    .unwrap();
    for (viewer, expected) in [
        (
            Some(OWNER_ID),
            vec!["bound_attachment", "unbound_attachment"],
        ),
        (Some("media_viewer"), vec!["bound_attachment"]),
        (None, vec!["bound_attachment"]),
    ] {
        let attachments = tokio::time::timeout(
            Duration::from_secs(2),
            media.list_request_attachments_for_viewer("list_request", viewer),
        )
        .await
        .expect("listing metadata must not read chunk inventory")
        .unwrap();
        assert_eq!(
            attachments
                .iter()
                .map(|value| value.attachment.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            attachments
                .iter()
                .all(|value| value.attachment.original.is_some())
        );
    }
    held.rollback().await.unwrap();
}

#[tokio::test]
async fn retry_replay_requires_attachment_visibility_and_survives_request_close() {
    let fixture = fixture();
    add_viewer(&fixture).await;
    start_request(&fixture, "retry_request", "retry-request", 1).await;
    upload_attachment(&fixture, "retry_request", "retry_attachment", 10).await;
    fixture
        .store
        .requests()
        .mutate_request_for_tests("retry_request", |request| {
            request.submitted_at_unix = Some(14);
            request.updated_at_unix = 14;
        })
        .await
        .unwrap();
    let media = fixture.store.media();
    media
        .retry_request_attachment_processing(
            "retry_request",
            "retry_attachment",
            OWNER_ID,
            "retry_operation",
            15,
        )
        .await
        .unwrap();
    assert!(
        media
            .request_attachment_for_viewer(
                "retry_request",
                "retry_attachment",
                Some("media_viewer")
            )
            .await
            .unwrap()
            .is_none()
    );
    let denied = media
        .retry_request_attachment_processing(
            "retry_request",
            "retry_attachment",
            "media_viewer",
            "retry_operation",
            16,
        )
        .await
        .unwrap_err();
    assert_eq!(denied.kind, PostgresErrorKind::NotFound);
    fixture
        .store
        .requests()
        .close_request(
            CloseRequestCommand {
                request_id: "retry_request".into(),
                actor_user_id: OWNER_ID.into(),
                event_id: "close_retry".into(),
                now_unix: 17,
            },
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    let replay = media
        .retry_request_attachment_processing(
            "retry_request",
            "retry_attachment",
            OWNER_ID,
            "retry_operation",
            18,
        )
        .await
        .unwrap();
    assert_eq!(replay.state, RequestAttachmentState::Processing);
}
