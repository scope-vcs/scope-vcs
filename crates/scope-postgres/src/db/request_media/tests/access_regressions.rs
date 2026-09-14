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
async fn attachment_access_controls_lists_and_retry_replay() {
    let fixture = fixture();
    add_viewer(&fixture).await;
    start_request(&fixture, "retry_request", 1).await;
    upload_attachment(&fixture, "retry_request", "bound_attachment", 10).await;
    upload_attachment(&fixture, "retry_request", "retry_attachment", 20).await;
    fixture
        .store
        .requests()
        .edit_request_identity(EditRequestIdentityCommand {
            request_id: "retry_request".into(),
            actor_user_id: OWNER_ID.into(),
            event_id: "bind".into(),
            title: None,
            description_markdown: Some(
                "![attachment](/request-attachments/bound_attachment)".into(),
            ),
            expected_description_markdown: Some(String::new()),
            now_unix: 30,
        })
        .await
        .unwrap();
    let media = fixture.store.media();
    assert!(
        media
            .list_request_attachments_for_viewer("retry_request", Some("media_viewer"))
            .await
            .unwrap()
            .is_empty()
    );
    fixture
        .store
        .requests()
        .mutate_request_for_tests("retry_request", |request| {
            request.submitted_at_unix = Some(31);
            request.updated_at_unix = 31;
        })
        .await
        .unwrap();
    for (viewer, expected) in [
        (
            Some(OWNER_ID),
            &["bound_attachment", "retry_attachment"][..],
        ),
        (Some("media_viewer"), &["bound_attachment"][..]),
        (None, &["bound_attachment"][..]),
    ] {
        let attachments = media
            .list_request_attachments_for_viewer("retry_request", viewer)
            .await
            .unwrap();
        assert_eq!(
            attachments
                .iter()
                .map(|value| value.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }
    media
        .retry_request_attachment_processing(
            "retry_request",
            "retry_attachment",
            OWNER_ID,
            "retry_operation",
            32,
        )
        .await
        .unwrap();
    let denied = media
        .retry_request_attachment_processing(
            "retry_request",
            "retry_attachment",
            "media_viewer",
            "retry_operation",
            33,
        )
        .await
        .unwrap_err();
    assert_eq!(denied.kind, PostgresErrorKind::NotFound);
    close_request(&fixture, "retry_request", "close_retry", 34).await;
    media
        .retry_request_attachment_processing(
            "retry_request",
            "retry_attachment",
            OWNER_ID,
            "retry_operation",
            35,
        )
        .await
        .unwrap();
}
