use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_api_role_authorizes_stores_and_serves_an_upload() {
    let Ok(admin_url) = std::env::var("SCOPE_MEDIA_ROLE_TEST_ADMIN_URL") else {
        return;
    };
    let media_api_url = std::env::var("SCOPE_MEDIA_ROLE_TEST_MEDIA_API_URL")
        .expect("media API URL must accompany the admin URL");
    let fixture = seeded_fixture(MetadataStore::connect(admin_url).await.unwrap());
    let media_api = MetadataStore::connect(media_api_url).await.unwrap().media();
    start_request(&fixture, "role_request", "role-request", 1).await;
    let prepared = prepare_attachment(&fixture, "role_request", "role_attachment", 10).await;

    let authorized = media_api
        .request_attachment_for_viewer("role_request", "role_attachment", Some(OWNER_ID))
        .await
        .unwrap()
        .expect("the uploader is authorized for its own attachment");
    assert_eq!(authorized.upload_id, prepared.upload_id);
    let released_part = part("role_attachment", "released");
    assert_eq!(
        media_api
            .reserve_upload_part(
                "role_attachment",
                &prepared.upload_id,
                OWNER_ID,
                released_part.clone(),
                "write_role",
                11,
                20,
            )
            .await
            .unwrap(),
        ReserveUploadPartResult::Write(released_part)
    );
    media_api
        .release_upload_part_write("role_attachment", 1, "write_role", 12)
        .await
        .unwrap();
    let stored_part = part("role_attachment", "stored");
    assert_eq!(
        media_api
            .reserve_upload_part(
                "role_attachment",
                &prepared.upload_id,
                OWNER_ID,
                stored_part.clone(),
                "write_role_retry",
                13,
                20,
            )
            .await
            .unwrap(),
        ReserveUploadPartResult::Write(stored_part.clone())
    );
    assert_eq!(
        media_api
            .mark_upload_part_stored(
                "role_attachment",
                &prepared.upload_id,
                OWNER_ID,
                1,
                &stored_part.object_key,
                "write_role_retry",
                14,
            )
            .await
            .unwrap(),
        StorePartResult::Recorded
    );
    fixture
        .store
        .media()
        .finish_request_attachment_upload(FinishRequestAttachmentUploadCommand {
            request_id: "role_request".to_string(),
            attachment_id: "role_attachment".to_string(),
            upload_id: prepared.upload_id,
            actor_user_id: OWNER_ID.to_string(),
            parts: vec![stored_part.receipt],
            now_unix: 15,
        })
        .await
        .unwrap();

    let (attachment, _) = media_api
        .request_attachment_media_for_viewer(
            "role_request",
            "role_attachment",
            Some(OWNER_ID),
            RequestMediaObjectTarget::Original,
        )
        .await
        .unwrap()
        .expect("the uploader can read its finished attachment");
    assert_eq!(attachment.id, "role_attachment");
}
