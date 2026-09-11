use super::*;
use crate::{
    db::{
        CatalogFixture, CloseRequestCommand, EditRequestIdentityCommand, MetadataStore,
        TestDatabaseTarget,
    },
    error::PostgresErrorKind,
};
use scope_domain::{
    account::UserAccount,
    policy::Visibility,
    repository::{RepoLifecycleState, Repository},
    requests::{
        RequestActorRole, RequestAudience, StartRequestInput,
        attachments::{
            RequestAttachmentCleanupLease, RequestAttachmentPartReceipt,
            RequestAttachmentProcessingLease, RequestAttachmentTarget,
        },
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};
use std::{ops::Range, time::Duration};

const OWNER_ID: &str = "media_owner";
const SOURCE_SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Fixture {
    store: MetadataStore,
    repository_id: String,
}

fn fixture() -> Fixture {
    let target = TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    let owner = UserAccount {
        id: OWNER_ID.to_string(),
        handle: "media-owner".to_string(),
        email: "media-owner@example.com".to_string(),
        email_verified: true,
    };
    let mut repository =
        Repository::new(&owner, "media", Visibility::Public, "repoi_media_tests").unwrap();
    repository.record.lifecycle_state = RepoLifecycleState::Ready;
    let repository_id = repository.record.id.clone();
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog
        .repositories
        .insert(repository_id.clone(), repository);
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    Fixture {
        store,
        repository_id,
    }
}

async fn start_request(fixture: &Fixture, request_id: &str, now_unix: u64) {
    let name = request_id.replace('_', "-");
    fixture
        .store
        .requests()
        .start_request(StartRequestInput {
            id: request_id.to_string(),
            repo_id: fixture.repository_id.clone(),
            name: name.clone(),
            author_user_id: OWNER_ID.to_string(),
            title: Some(format!("Request {name}")),
            author_role: RequestActorRole::Owner,
            audience: RequestAudience::Public,
            base_main_oid: "base".to_string(),
            event_id: format!("event_start_{request_id}"),
            now_unix,
        })
        .await
        .unwrap();
}

async fn prepare_attachment(
    fixture: &Fixture,
    request_id: &str,
    attachment_id: &str,
    now_unix: u64,
) -> PreparedRequestAttachment {
    fixture
        .store
        .media()
        .prepare_request_attachment(
            prepare_command(request_id, attachment_id, now_unix),
            default_limits(),
        )
        .await
        .unwrap()
}

fn prepare_command(
    request_id: &str,
    attachment_id: &str,
    now_unix: u64,
) -> PrepareRequestAttachmentCommand {
    PrepareRequestAttachmentCommand {
        attachment_id: attachment_id.into(),
        upload_id: format!("upload_{attachment_id}"),
        operation_id: format!("operation_{attachment_id}"),
        request_id: request_id.into(),
        actor_user_id: OWNER_ID.into(),
        target: RequestAttachmentTarget::Description,
        filename: format!("{attachment_id}.png"),
        declared_media_type: "image/png".into(),
        size_bytes: 4,
        sha256: SOURCE_SHA256.into(),
        now_unix,
    }
}

fn part(attachment_id: &str, suffix: &str) -> StoredRequestAttachmentPart {
    StoredRequestAttachmentPart {
        receipt: RequestAttachmentPartReceipt {
            part_number: 1,
            size_bytes: 4,
            sha256: SOURCE_SHA256.to_string(),
        },
        object_key: format!("media/v1/staged/{attachment_id}/{suffix}"),
    }
}

async fn reserve_part(
    fixture: &Fixture,
    prepared: &PreparedRequestAttachment,
    stored_part: &StoredRequestAttachmentPart,
    token: &str,
    lease: Range<u64>,
) -> ReserveUploadPartResult {
    fixture
        .store
        .media()
        .reserve_upload_part(
            &prepared.attachment.id,
            &prepared.upload_id,
            OWNER_ID,
            stored_part.clone(),
            token,
            lease.start,
            lease.end,
        )
        .await
        .unwrap()
}

async fn store_part(
    fixture: &Fixture,
    prepared: &PreparedRequestAttachment,
    stored_part: &StoredRequestAttachmentPart,
    token: &str,
    now_unix: u64,
) -> StorePartResult {
    fixture
        .store
        .media()
        .mark_upload_part_stored(
            &prepared.attachment.id,
            &prepared.upload_id,
            OWNER_ID,
            stored_part.receipt.part_number,
            &stored_part.object_key,
            token,
            now_unix,
        )
        .await
        .unwrap()
}

async fn claim_processing(
    fixture: &Fixture,
    token: &str,
    now_unix: u64,
    expires_at_unix: u64,
) -> RequestAttachmentProcessingLease {
    fixture
        .store
        .media()
        .claim_processing_job(token, now_unix, expires_at_unix)
        .await
        .unwrap()
        .unwrap()
}

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

async fn upload_attachment(
    fixture: &Fixture,
    request_id: &str,
    attachment_id: &str,
    now_unix: u64,
) {
    let prepared = prepare_attachment(fixture, request_id, attachment_id, now_unix).await;
    let stored_part = part(attachment_id, "part-1");
    assert_eq!(
        reserve_part(
            fixture,
            &prepared,
            &stored_part,
            &format!("write_{attachment_id}"),
            now_unix + 1..now_unix + 10,
        )
        .await,
        ReserveUploadPartResult::Write(stored_part.clone())
    );
    assert_eq!(
        store_part(
            fixture,
            &prepared,
            &stored_part,
            &format!("write_{attachment_id}"),
            now_unix + 2,
        )
        .await,
        StorePartResult::Recorded
    );
    fixture
        .store
        .media()
        .finish_request_attachment_upload(FinishRequestAttachmentUploadCommand {
            request_id: request_id.to_string(),
            attachment_id: attachment_id.to_string(),
            upload_id: prepared.upload_id,
            actor_user_id: OWNER_ID.to_string(),
            parts: vec![stored_part.receipt],
            now_unix: now_unix + 3,
        })
        .await
        .unwrap();
}

async fn close_request(fixture: &Fixture, request_id: &str, event_id: &str, now_unix: u64) {
    fixture
        .store
        .requests()
        .close_request(
            CloseRequestCommand {
                request_id: request_id.into(),
                actor_user_id: OWNER_ID.into(),
                event_id: event_id.into(),
                now_unix,
            },
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
}

async fn reserve_processing_key(
    fixture: &Fixture,
    lease: &RequestAttachmentProcessingLease,
    object_key: &str,
    now_unix: u64,
) {
    assert_eq!(
        fixture
            .store
            .media()
            .reserve_processing_object_key(
                &lease.attachment_id,
                &lease.lease_token,
                lease.lease_generation,
                object_key,
                now_unix,
            )
            .await
            .unwrap(),
        MediaLeaseMutation::Applied(())
    );
}

async fn complete_processing_without_derivatives(
    fixture: &Fixture,
    lease: RequestAttachmentProcessingLease,
    now_unix: u64,
) -> MediaLeaseMutation<scope_domain::requests::attachments::RequestAttachment> {
    fixture
        .store
        .media()
        .complete_processing_job(CompleteRequestAttachmentProcessingCommand {
            attachment_id: lease.attachment_id,
            lease_token: lease.lease_token,
            lease_generation: lease.lease_generation,
            source: ValidatedRequestAttachmentSource {
                detected_media_type: "image/png".into(),
                size_bytes: 4,
                sha256: SOURCE_SHA256.into(),
                width: Some(1),
                height: Some(1),
                duration_millis: None,
            },
            derivatives: Vec::new(),
            now_unix,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn upload_part_reservation_is_idempotent_and_fences_expired_writers() {
    let fixture = fixture();
    start_request(&fixture, "lease_request", 1).await;
    let prepared = prepare_attachment(&fixture, "lease_request", "lease_attachment", 10).await;
    let first = part("lease_attachment", "first");
    let replacement = part("lease_attachment", "replacement");

    assert_eq!(
        reserve_part(&fixture, &prepared, &first, "first-token", 11..20,).await,
        ReserveUploadPartResult::Write(first.clone())
    );
    assert_eq!(
        reserve_part(&fixture, &prepared, &replacement, "competing-token", 12..21,).await,
        ReserveUploadPartResult::Busy
    );
    assert_eq!(
        reserve_part(
            &fixture,
            &prepared,
            &replacement,
            "replacement-token",
            20..30,
        )
        .await,
        ReserveUploadPartResult::Write(replacement.clone())
    );
    assert_eq!(
        store_part(&fixture, &prepared, &first, "first-token", 21).await,
        StorePartResult::WriteLeaseLost
    );
    assert_eq!(
        store_part(&fixture, &prepared, &replacement, "replacement-token", 21).await,
        StorePartResult::Recorded
    );
    assert_eq!(
        store_part(&fixture, &prepared, &replacement, "replacement-token", 22).await,
        StorePartResult::AlreadyRecorded(replacement.clone())
    );
    assert_eq!(
        reserve_part(
            &fixture,
            &prepared,
            &part("lease_attachment", "ignored-after-store"),
            "ignored-token",
            22..31,
        )
        .await,
        ReserveUploadPartResult::Stored(replacement)
    );

    let abandoned = fixture
        .store
        .db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT object_key FROM scope_request_media_abandoned_objects WHERE attachment_id = $1",
            ["lease_attachment".into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        abandoned.try_get::<String>("", "object_key").unwrap(),
        first.object_key
    );
}

#[tokio::test]
async fn cross_request_binding_rejection_rolls_back_identity_event_and_prior_binding() {
    let fixture = fixture();
    start_request(&fixture, "binding_request", 1).await;
    start_request(&fixture, "other_request", 2).await;
    upload_attachment(&fixture, "binding_request", "binding_attachment", 10).await;
    upload_attachment(&fixture, "other_request", "other_attachment", 20).await;

    let original_markdown = "![attachment](/request-attachments/binding_attachment)".to_string();
    fixture
        .store
        .requests()
        .edit_request_identity(EditRequestIdentityCommand {
            request_id: "binding_request".into(),
            actor_user_id: OWNER_ID.into(),
            event_id: "event_bind_original".into(),
            title: None,
            description_markdown: Some(original_markdown.clone()),
            expected_description_markdown: Some(String::new()),
            now_unix: 30,
        })
        .await
        .unwrap();

    let error = fixture
        .store
        .requests()
        .edit_request_identity(EditRequestIdentityCommand {
            request_id: "binding_request".to_string(),
            actor_user_id: OWNER_ID.to_string(),
            event_id: "event_invalid_cross_request".to_string(),
            title: Some("This must roll back".to_string()),
            description_markdown: Some(
                "![attachment](/request-attachments/other_attachment)".into(),
            ),
            expected_description_markdown: Some(original_markdown.clone()),
            now_unix: 31,
        })
        .await
        .unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::PermissionDenied);
    assert!(error.message.contains("belongs to another request"));

    let request = fixture
        .store
        .requests()
        .request_for_tests("binding_request")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.description_markdown, original_markdown);
    assert!(
        fixture
            .store
            .requests()
            .request_events_for_tests()
            .await
            .unwrap()
            .iter()
            .all(|event| event.id != "event_invalid_cross_request")
    );
    assert_eq!(
        super::persistence::bindings_for_attachment(
            fixture.store.db.as_ref(),
            "binding_attachment",
        )
        .await
        .unwrap()
        .len(),
        1
    );
}

#[tokio::test]
async fn processing_lease_takeover_orphans_old_output_and_fences_stale_completion() {
    let fixture = fixture();
    start_request(&fixture, "processing_request", 1).await;
    upload_attachment(&fixture, "processing_request", "processing_attachment", 10).await;

    let first_lease = claim_processing(&fixture, "first-processing-token", 20, 30).await;
    let old_output_key = "media/v1/staged/processing_attachment/old-output";
    reserve_processing_key(&fixture, &first_lease, old_output_key, 21).await;

    let replacement_lease =
        claim_processing(&fixture, "replacement-processing-token", 30, 40).await;
    assert_eq!(
        replacement_lease.lease_generation,
        first_lease.lease_generation + 1
    );
    assert!(matches!(
        complete_processing_without_derivatives(&fixture, first_lease, 31).await,
        MediaLeaseMutation::LeaseLost
    ));

    let old_output = fixture
        .store
        .db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT state FROM scope_request_media_processing_objects WHERE object_key = $1",
            [old_output_key.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        old_output.try_get::<String>("", "state").unwrap(),
        "Orphaned"
    );
}

#[tokio::test]
async fn completed_manifests_reject_mutation() {
    let fixture = fixture();
    start_request(&fixture, "manifest_request", 1).await;
    upload_attachment(&fixture, "manifest_request", "manifest_attachment", 10).await;

    let error = fixture
        .store
        .db
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_request_media_manifests SET media_type = $2 WHERE id = $1",
            ["manifest_attachment:original".into(), "image/jpeg".into()],
        ))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("completed request media manifests are immutable")
    );
}

#[tokio::test]
async fn request_deletion_fences_processing_and_reconciles_every_known_object_key() {
    let fixture = fixture();
    start_request(&fixture, "deleted_request", 1).await;
    upload_attachment(&fixture, "deleted_request", "deleted_attachment", 10).await;

    let processing_lease = claim_processing(&fixture, "processing-before-delete", 20, 30).await;
    let late_output_key = "media/v1/staged/deleted_attachment/late-output";
    reserve_processing_key(&fixture, &processing_lease, late_output_key, 21).await;

    close_request(&fixture, "deleted_request", "event_delete_request", 22).await;
    assert!(matches!(
        complete_processing_without_derivatives(&fixture, processing_lease, 23).await,
        MediaLeaseMutation::LeaseLost
    ));

    let cleanup = claim_cleanup(&fixture, "cleanup-after-grace", 90, 100).await;
    assert!(cleanup.object_keys.iter().any(|key| key == late_output_key));
    assert!(
        cleanup
            .object_keys
            .iter()
            .any(|key| key.ends_with("/part-1"))
    );
    assert_eq!(
        fixture
            .store
            .media()
            .complete_cleanup_job(
                "deleted_attachment",
                &cleanup.lease_token,
                cleanup.lease_generation,
                91,
            )
            .await
            .unwrap(),
        MediaLeaseMutation::Applied(())
    );

    let reconciliation_time = 91 + 24 * 60 * 60;
    fixture
        .store
        .media()
        .enqueue_expired_attachment_cleanup(reconciliation_time)
        .await
        .unwrap();
    let reconciled = claim_cleanup(
        &fixture,
        "cleanup-reconciliation",
        reconciliation_time,
        reconciliation_time + 10,
    )
    .await;
    assert!(
        reconciled
            .object_keys
            .iter()
            .any(|key| key == late_output_key)
    );
}

#[tokio::test]
async fn binding_a_ready_attachment_notifies_only_after_the_transaction_commits() {
    let fixture = fixture();
    start_request(&fixture, "notification_request", 1).await;
    upload_attachment(
        &fixture,
        "notification_request",
        "notification_attachment",
        10,
    )
    .await;
    fixture
        .store
        .db
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_request_media_attachments SET state = 'Ready' WHERE id = $1",
            ["notification_attachment".into()],
        ))
        .await
        .unwrap();

    let mut listener =
        sqlx::postgres::PgListener::connect_with(fixture.store.db.get_postgres_connection_pool())
            .await
            .unwrap();
    listener.listen("scope_repo_changes").await.unwrap();
    let tx = fixture.store.db.begin().await.unwrap();
    replace_bindings_for_markdown(
        &tx,
        "notification_request",
        OWNER_ID,
        &scope_domain::requests::attachments::RequestAttachmentBindingTarget::Description,
        "![attachment](/request-attachments/notification_attachment)",
        30,
    )
    .await
    .unwrap();

    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.recv())
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
    let notification = tokio::time::timeout(Duration::from_secs(2), listener.recv())
        .await
        .expect("binding commit should publish a repository change")
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(notification.payload()).unwrap();
    assert_eq!(
        payload["event"]["kind"]["RequestAttachmentChanged"],
        serde_json::json!({
            "request_id": "notification_request",
            "attachment_id": "notification_attachment",
            "audience": "Public",
        })
    );
}

#[tokio::test]
async fn expired_upload_operations_report_expiry_before_and_after_cleanup_discovery() {
    let fixture = fixture();
    start_request(&fixture, "expiry_request", 1).await;
    let prepared = prepare_attachment(&fixture, "expiry_request", "expired_attachment", 10).await;
    let expires = prepared.attachment.upload_expires_at_unix;
    let command = |operation_id: &str| PrepareRequestAttachmentCommand {
        upload_id: "replacement_upload".into(),
        operation_id: operation_id.into(),
        filename: "expired_attachment.png".into(),
        ..prepare_command("expiry_request", "replacement_attachment", expires)
    };
    for cleanup_discovered in [false, true] {
        if cleanup_discovered {
            fixture
                .store
                .media()
                .enqueue_expired_attachment_cleanup(expires)
                .await
                .unwrap();
        }
        let error = fixture
            .store
            .media()
            .prepare_request_attachment(command("operation_expired_attachment"), default_limits())
            .await
            .unwrap_err();
        assert_eq!(error.kind, PostgresErrorKind::AttachmentUploadExpired);
    }
    let replacement = fixture
        .store
        .media()
        .prepare_request_attachment(command("replacement_operation"), default_limits())
        .await
        .unwrap();
    assert_ne!(replacement.attachment.id, prepared.attachment.id);
}

mod access_regressions;
mod cleanup_regressions;
