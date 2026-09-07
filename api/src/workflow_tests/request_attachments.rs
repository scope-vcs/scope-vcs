use super::requests::api_request;
use super::*;
use scope_api_contract::attachments::{
    PrepareRequestAttachmentResponse, RequestAttachmentResponse,
};
use scope_domain::requests::attachments::RequestAttachmentPartReceipt;
use scope_postgres::db::StoredRequestAttachmentPart;

const ROOT: &str = "/v1/repos/owner/repo/requests/req_media/attachments";

async fn fixture() -> AppState {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    requests::create_owner_request(
        &state,
        "req_media",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .await;
    state
}

fn prepare_body(operation_id: &str) -> String {
    serde_json::json!({
        "operation_id": operation_id,
        "target": { "kind": "Description" },
        "filename": "phone.jpg",
        "declared_media_type": "image/jpeg",
        "size_bytes": 4,
        "sha256": "a".repeat(64),
    })
    .to_string()
}

async fn prepare(state: &AppState, operation: &str) -> PrepareRequestAttachmentResponse {
    let response = api_request(
        router(state.clone()),
        "POST",
        &format!("{ROOT}/prepare"),
        Some(&bearer_header()),
        Some(&prepare_body(operation)),
    )
    .await;
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

async fn finish(
    state: &AppState,
    prepared: &PrepareRequestAttachmentResponse,
) -> RequestAttachmentResponse {
    let receipt = RequestAttachmentPartReceipt {
        part_number: 1,
        size_bytes: 4,
        sha256: "a".repeat(64),
    };
    let now = crate::persistence::unix_now().unwrap();
    state
        .metadata
        .media()
        .reserve_upload_part(
            &prepared.attachment.id,
            &prepared.transfer.upload_id,
            &test_owner_id(),
            StoredRequestAttachmentPart {
                receipt: receipt.clone(),
                object_key: "encrypted-test-part".into(),
            },
            "api-test-part-write",
            now,
            now + 60,
        )
        .await
        .unwrap();
    state
        .metadata
        .media()
        .mark_upload_part_stored(
            &prepared.attachment.id,
            &prepared.transfer.upload_id,
            &test_owner_id(),
            1,
            "encrypted-test-part",
            "api-test-part-write",
            now,
        )
        .await
        .unwrap();
    let body = serde_json::json!({"upload_id": prepared.transfer.upload_id, "parts": [receipt]})
        .to_string();
    let response = api_request(
        router(state.clone()),
        "POST",
        &format!("{ROOT}/{}/finish", prepared.attachment.id),
        Some(&bearer_header()),
        Some(&body),
    )
    .await;
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

async fn validate_source(state: &AppState, attachment_id: &str) {
    use scope_postgres::db::{
        ValidateRequestAttachmentSourceCommand, ValidatedRequestAttachmentSource,
    };
    let now = crate::persistence::unix_now().unwrap();
    let media = state.metadata.media();
    let lease = media
        .claim_processing_job("test-lease", now, now + 60)
        .await
        .unwrap()
        .unwrap();
    media
        .mark_processing_source_validated(ValidateRequestAttachmentSourceCommand {
            attachment_id: attachment_id.into(),
            lease_token: lease.lease_token,
            lease_generation: lease.lease_generation,
            source: ValidatedRequestAttachmentSource {
                detected_media_type: "image/jpeg".into(),
                size_bytes: 4,
                sha256: "a".repeat(64),
                width: Some(2),
                height: Some(2),
                duration_millis: None,
            },
            now_unix: now,
        })
        .await
        .unwrap();
}

async fn add_writer(state: &AppState) -> String {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        bearer_header_for("user_writer", "writer@example.com")
            .parse()
            .unwrap(),
    );
    let writer = crate::auth::scope::require_scope_user(state, &headers)
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.members.push(test_repository_member(
                TEST_REPO_ID,
                writer.id.clone(),
                member_permissions(true, false, false),
            ));
        })
        .await
        .unwrap();
    writer.id
}

#[tokio::test]
async fn request_attachment_prepare_resumes_receipts_and_hides_unvalidated_original() {
    let state = fixture().await;
    let first = prepare(&state, "resume-photo").await;
    assert!(first.transfer.acknowledged_parts.is_empty());
    let uploaded = finish(&state, &first).await;
    let resumed = prepare(&state, "resume-photo").await;
    assert_eq!(resumed.attachment.id, first.attachment.id);
    assert_eq!(resumed.transfer.upload_id, first.transfer.upload_id);
    assert_eq!(resumed.transfer.acknowledged_parts.len(), 1);
    assert_eq!(uploaded.id, first.attachment.id);

    let response = api_request(
        router(state.clone()),
        "POST",
        &format!("{ROOT}/{}/media-grant", uploaded.id),
        Some(&bearer_header()),
        Some(r#"{"target":{"kind":"original"}}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = api_request(
        router(state),
        "GET",
        &format!("{ROOT}/{}", uploaded.id),
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn request_attachment_original_grant_requires_fenced_source_validation() {
    use jsonwebtoken::{DecodingKey, Validation, decode};
    use scope_api_contract::attachments::{
        CreateRequestAttachmentMediaGrantResponse, RequestAttachmentMediaGrantClaims,
    };
    let state = fixture().await;
    let prepared = prepare(&state, "validated-original").await;
    finish(&state, &prepared).await;
    let now = crate::persistence::unix_now().unwrap();
    validate_source(&state, &prepared.attachment.id).await;
    let response = api_request(
        router(state),
        "POST",
        &format!("{ROOT}/{}/media-grant", prepared.attachment.id),
        Some(&bearer_header()),
        Some(r#"{"target":{"kind":"original"}}"#),
    )
    .await;
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.to_string().contains("encrypted-test-part"));
    let grant: CreateRequestAttachmentMediaGrantResponse = serde_json::from_value(body).unwrap();
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    let claims = decode::<RequestAttachmentMediaGrantClaims>(
        &grant.grant,
        &DecodingKey::from_ed_pem(crate::cache_grants::TEST_PUBLIC_KEY.as_bytes()).unwrap(),
        &validation,
    )
    .unwrap()
    .claims;
    assert_eq!(claims.viewer_user_id, Some(test_owner_id()));
    assert_eq!(claims.request_id, "req_media");
    assert_eq!(claims.repository_id, TEST_REPO_ID);
    assert_eq!(claims.attachment_id, prepared.attachment.id);
    assert!(
        grant
            .media_url
            .starts_with("http://127.0.0.1:8083/v1/attachments/")
    );
    assert!(grant.expires_at_unix <= now + 301);
}

#[tokio::test]
async fn request_attachment_description_binding_is_atomic_and_checks_stale_text() {
    let state = fixture().await;
    let prepared = prepare(&state, "bind-photo").await;
    let markdown = format!(
        "Photo: ![phone](/request-attachments/{})",
        prepared.attachment.id
    );
    let update =
        serde_json::json!({"description_markdown": markdown, "expected_description_markdown": ""})
            .to_string();
    let uri = "/v1/repos/owner/repo/requests/req_media";
    let response = api_request(
        router(state.clone()),
        "PATCH",
        uri,
        Some(&bearer_header()),
        Some(&update),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    finish(&state, &prepared).await;
    let response = api_request(
        router(state.clone()),
        "PATCH",
        uri,
        Some(&bearer_header()),
        Some(&update),
    )
    .await;
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let bound = state
        .metadata
        .media()
        .request_attachment_for_viewer("req_media", &prepared.attachment.id, Some(&test_owner_id()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bound.bindings.len(), 1);

    let stale = serde_json::json!({"description_markdown": "stale edit", "expected_description_markdown": ""}).to_string();
    let response = api_request(
        router(state.clone()),
        "PATCH",
        uri,
        Some(&bearer_header()),
        Some(&stale),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let request = state
        .metadata
        .requests()
        .request_by_id("req_media")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.description_markdown, markdown);
}

#[tokio::test]
async fn request_attachment_events_hide_unbound_uploads_from_other_maintainers() {
    use tokio_stream::StreamExt;
    let state = fixture().await;
    add_writer(&state).await;
    let prepared = prepare(&state, "private-draft-event").await;
    let response = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/events",
        Some(&bearer_header_for("user_writer", "writer@example.com")),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    assert!(
        String::from_utf8(stream.next().await.unwrap().unwrap().to_vec())
            .unwrap()
            .contains("Connected")
    );
    let incarnation = test_repo_incarnation();
    let event = scope_api_contract::RepoChangeEvent {
        repo_id: incarnation.repository_id().into(),
        incarnation_id: incarnation.incarnation_id().into(),
        version: 0,
        kind: scope_api_contract::RepoChangeKind::RequestAttachmentChanged {
            request_id: "req_media".into(),
            attachment_id: prepared.attachment.id.clone(),
            audience: scope_api_contract::RequestAudience::Private,
        },
    };
    state.repo_events.publish_event(event.clone());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .is_err()
    );

    finish(&state, &prepared).await;
    let body = serde_json::json!({"description_markdown": format!("![photo](/request-attachments/{})", prepared.attachment.id)}).to_string();
    let response = api_request(
        router(state.clone()),
        "PATCH",
        "/v1/repos/owner/repo/requests/req_media",
        Some(&bearer_header()),
        Some(&body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    state.repo_events.publish_event(event);
    let received = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let message =
                String::from_utf8(stream.next().await.unwrap().unwrap().to_vec()).unwrap();
            if message.contains("RequestAttachmentChanged") {
                break message;
            }
        }
    })
    .await
    .unwrap();
    assert!(received.contains(&prepared.attachment.id));
}

#[tokio::test]
async fn request_attachment_upload_ownership_does_not_survive_revoked_request_access() {
    let state = fixture().await;
    let writer_id = add_writer(&state).await;
    let auth = bearer_header_for("user_writer", "writer@example.com");
    let response = api_request(
        router(state.clone()),
        "POST",
        &format!("{ROOT}/prepare"),
        Some(&auth),
        Some(&prepare_body("revoked-uploader")),
    )
    .await;
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let prepared: PrepareRequestAttachmentResponse = serde_json::from_value(body).unwrap();
    assert_eq!(prepared.attachment.uploader_user_id, writer_id);
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.members.retain(|member| member.user_id != writer_id);
        })
        .await
        .unwrap();
    assert!(
        state
            .metadata
            .media()
            .request_attachment_for_viewer("req_media", &prepared.attachment.id, Some(&writer_id),)
            .await
            .unwrap()
            .is_none()
    );
    let response = api_request(
        router(state),
        "GET",
        &format!("{ROOT}/{}", prepared.attachment.id),
        Some(&auth),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn public_attachment_requires_a_published_binding_and_exact_repository_path() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    requests::create_public_request(
        &state,
        "req_media",
        test_owner_id(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .await;
    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_media", |request| {
            request.submitted_at_unix = Some(4);
            request.updated_at_unix = 4;
        })
        .await
        .unwrap();
    let prepared = prepare(&state, "public-bound-photo").await;
    finish(&state, &prepared).await;
    validate_source(&state, &prepared.attachment.id).await;
    let grant_path = format!("{ROOT}/{}/media-grant", prepared.attachment.id);
    let response = api_request(
        router(state.clone()),
        "POST",
        &grant_path,
        None,
        Some(r#"{"target":{"kind":"original"}}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let update = serde_json::json!({
        "description_markdown": format!("![public photo](/request-attachments/{})", prepared.attachment.id),
        "expected_description_markdown": "",
    }).to_string();
    let response = api_request(
        router(state.clone()),
        "PATCH",
        "/v1/repos/owner/repo/requests/req_media",
        Some(&bearer_header()),
        Some(&update),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = api_request(
        router(state.clone()),
        "POST",
        &grant_path,
        None,
        Some(r#"{"target":{"kind":"original"}}"#),
    )
    .await;
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["media_url"]
            .as_str()
            .unwrap()
            .contains(&prepared.attachment.id)
    );

    let response = api_request(
        router(state),
        "POST",
        &grant_path.replace("owner/repo", "owner/wrong-repo"),
        Some(&bearer_header()),
        Some(r#"{"target":{"kind":"original"}}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
