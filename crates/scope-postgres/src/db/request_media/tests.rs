use super::*;
use crate::{
    db::{CatalogFixture, MetadataStore, TestDatabaseTarget},
    error::PostgresErrorKind,
};
use scope_domain::{
    account::UserAccount,
    policy::Visibility,
    repository::{RepoLifecycleState, Repository},
    requests::{
        CloseRequestInput, EditRequestIdentityInput, RequestActorRole, RequestAudience,
        StartRequestInput,
        attachments::{
            RequestAttachmentDerivative, RequestAttachmentDerivativeKind,
            RequestAttachmentPartReceipt, RequestAttachmentState, RequestAttachmentStoredObject,
            RequestAttachmentTarget,
        },
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};
use std::time::Duration;

const OWNER_ID: &str = "media_owner";
const SOURCE_SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Fixture {
    store: MetadataStore,
    target: TestDatabaseTarget,
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
        target,
        repository_id,
    }
}

async fn start_request(fixture: &Fixture, request_id: &str, name: &str, now_unix: u64) {
    fixture
        .store
        .requests()
        .start_request(StartRequestInput {
            id: request_id.to_string(),
            repo_id: fixture.repository_id.clone(),
            name: name.to_string(),
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
            PrepareRequestAttachmentCommand {
                attachment_id: attachment_id.to_string(),
                upload_id: format!("upload_{attachment_id}"),
                operation_id: format!("operation_{attachment_id}"),
                request_id: request_id.to_string(),
                actor_user_id: OWNER_ID.to_string(),
                target: RequestAttachmentTarget::Description,
                filename: format!("{attachment_id}.png"),
                declared_media_type: "image/png".to_string(),
                size_bytes: 4,
                sha256: SOURCE_SHA256.to_string(),
                now_unix,
            },
            default_limits(),
        )
        .await
        .unwrap()
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

async fn upload_attachment(
    fixture: &Fixture,
    request_id: &str,
    attachment_id: &str,
    now_unix: u64,
) {
    let prepared = prepare_attachment(fixture, request_id, attachment_id, now_unix).await;
    let stored_part = part(attachment_id, "part-1");
    assert_eq!(
        fixture
            .store
            .media()
            .reserve_upload_part(
                attachment_id,
                &prepared.upload_id,
                OWNER_ID,
                stored_part.clone(),
                &format!("write_{attachment_id}"),
                now_unix + 1,
                now_unix + 10,
            )
            .await
            .unwrap(),
        ReserveUploadPartResult::Write(stored_part.clone())
    );
    assert_eq!(
        fixture
            .store
            .media()
            .mark_upload_part_stored(
                attachment_id,
                &prepared.upload_id,
                OWNER_ID,
                1,
                &stored_part.object_key,
                &format!("write_{attachment_id}"),
                now_unix + 2,
            )
            .await
            .unwrap(),
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

fn attachment_markdown(attachment_id: &str) -> String {
    format!("![attachment](/request-attachments/{attachment_id})")
}

#[tokio::test]
async fn upload_part_reservation_is_idempotent_and_fences_expired_writers() {
    let fixture = fixture();
    start_request(&fixture, "lease_request", "lease-request", 1).await;
    let prepared = prepare_attachment(&fixture, "lease_request", "lease_attachment", 10).await;
    let first = part("lease_attachment", "first");
    let replacement = part("lease_attachment", "replacement");

    assert_eq!(
        fixture
            .store
            .media()
            .reserve_upload_part(
                "lease_attachment",
                &prepared.upload_id,
                OWNER_ID,
                first.clone(),
                "first-token",
                11,
                20,
            )
            .await
            .unwrap(),
        ReserveUploadPartResult::Write(first.clone())
    );
    assert_eq!(
        fixture
            .store
            .media()
            .reserve_upload_part(
                "lease_attachment",
                &prepared.upload_id,
                OWNER_ID,
                replacement.clone(),
                "competing-token",
                12,
                21,
            )
            .await
            .unwrap(),
        ReserveUploadPartResult::Busy
    );
    assert_eq!(
        fixture
            .store
            .media()
            .reserve_upload_part(
                "lease_attachment",
                &prepared.upload_id,
                OWNER_ID,
                replacement.clone(),
                "replacement-token",
                20,
                30,
            )
            .await
            .unwrap(),
        ReserveUploadPartResult::Write(replacement.clone())
    );
    assert_eq!(
        fixture
            .store
            .media()
            .mark_upload_part_stored(
                "lease_attachment",
                &prepared.upload_id,
                OWNER_ID,
                1,
                &first.object_key,
                "first-token",
                21,
            )
            .await
            .unwrap(),
        StorePartResult::WriteLeaseLost
    );
    assert_eq!(
        fixture
            .store
            .media()
            .mark_upload_part_stored(
                "lease_attachment",
                &prepared.upload_id,
                OWNER_ID,
                1,
                &replacement.object_key,
                "replacement-token",
                21,
            )
            .await
            .unwrap(),
        StorePartResult::Recorded
    );
    assert_eq!(
        fixture
            .store
            .media()
            .mark_upload_part_stored(
                "lease_attachment",
                &prepared.upload_id,
                OWNER_ID,
                1,
                &replacement.object_key,
                "replacement-token",
                22,
            )
            .await
            .unwrap(),
        StorePartResult::AlreadyRecorded(replacement.clone())
    );
    assert_eq!(
        fixture
            .store
            .media()
            .reserve_upload_part(
                "lease_attachment",
                &prepared.upload_id,
                OWNER_ID,
                part("lease_attachment", "ignored-after-store"),
                "ignored-token",
                22,
                31,
            )
            .await
            .unwrap(),
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
    start_request(&fixture, "binding_request", "binding-request", 1).await;
    start_request(&fixture, "other_request", "other-request", 2).await;
    upload_attachment(&fixture, "binding_request", "binding_attachment", 10).await;
    upload_attachment(&fixture, "other_request", "other_attachment", 20).await;

    let original_markdown = attachment_markdown("binding_attachment");
    fixture
        .store
        .requests()
        .edit_request_identity(EditRequestIdentityInput {
            request_id: "binding_request".to_string(),
            actor_user_id: OWNER_ID.to_string(),
            actor_can_edit_identity: false,
            event_id: "event_bind_original".to_string(),
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
        .edit_request_identity(EditRequestIdentityInput {
            request_id: "binding_request".to_string(),
            actor_user_id: OWNER_ID.to_string(),
            actor_can_edit_identity: false,
            event_id: "event_invalid_cross_request".to_string(),
            title: Some("This must roll back".to_string()),
            description_markdown: Some(attachment_markdown("other_attachment")),
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
    assert_eq!(request.title, "Request binding-request");
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
    assert!(
        super::persistence::bindings_for_attachment(fixture.store.db.as_ref(), "other_attachment",)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn processing_lease_takeover_orphans_old_output_and_fences_stale_completion() {
    let fixture = fixture();
    start_request(&fixture, "processing_request", "processing-request", 1).await;
    upload_attachment(&fixture, "processing_request", "processing_attachment", 10).await;

    let first_lease = fixture
        .store
        .media()
        .claim_processing_job("first-processing-token", 20, 30)
        .await
        .unwrap()
        .unwrap();
    let old_output_key = "media/v1/staged/processing_attachment/old-output";
    assert_eq!(
        fixture
            .store
            .media()
            .reserve_processing_object_key(
                "processing_attachment",
                &first_lease.lease_token,
                first_lease.lease_generation,
                old_output_key,
                21,
            )
            .await
            .unwrap(),
        MediaLeaseMutation::Applied(())
    );

    let replacement_lease = fixture
        .store
        .media()
        .claim_processing_job("replacement-processing-token", 30, 40)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        replacement_lease.lease_generation,
        first_lease.lease_generation + 1
    );
    let source = ValidatedRequestAttachmentSource {
        detected_media_type: "image/png".to_string(),
        size_bytes: 4,
        sha256: SOURCE_SHA256.to_string(),
        width: Some(1),
        height: Some(1),
        duration_millis: None,
    };
    assert!(matches!(
        fixture
            .store
            .media()
            .complete_processing_job(CompleteRequestAttachmentProcessingCommand {
                attachment_id: "processing_attachment".to_string(),
                lease_token: first_lease.lease_token,
                lease_generation: first_lease.lease_generation,
                source,
                derivatives: Vec::new(),
                now_unix: 31,
            })
            .await
            .unwrap(),
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
async fn successful_processing_completion_persists_derivative_with_repository_budget() {
    let fixture = fixture();
    start_request(&fixture, "completion_request", "completion-request", 1).await;
    upload_attachment(&fixture, "completion_request", "completion_attachment", 10).await;

    let lease = fixture
        .store
        .media()
        .claim_processing_job("completion-token", 20, 40)
        .await
        .unwrap()
        .unwrap();
    let output_key = "media/v1/staged/completion_attachment/preview";
    assert_eq!(
        fixture
            .store
            .media()
            .reserve_processing_object_key(
                "completion_attachment",
                &lease.lease_token,
                lease.lease_generation,
                output_key,
                21,
            )
            .await
            .unwrap(),
        MediaLeaseMutation::Applied(())
    );

    let derivative_sha256 = "b".repeat(64);
    let manifest_id = "manifest_completion_preview".to_string();
    let completed = fixture
        .store
        .media()
        .complete_processing_job(CompleteRequestAttachmentProcessingCommand {
            attachment_id: "completion_attachment".to_string(),
            lease_token: lease.lease_token,
            lease_generation: lease.lease_generation,
            source: ValidatedRequestAttachmentSource {
                detected_media_type: "image/png".to_string(),
                size_bytes: 4,
                sha256: SOURCE_SHA256.to_string(),
                width: Some(1),
                height: Some(1),
                duration_millis: None,
            },
            derivatives: vec![CompletedRequestAttachmentDerivative {
                derivative: RequestAttachmentDerivative {
                    id: "derivative_completion_preview".to_string(),
                    kind: RequestAttachmentDerivativeKind::ImagePreview,
                    media_type: "image/webp".to_string(),
                    object: RequestAttachmentStoredObject {
                        object_key: manifest_id.clone(),
                        size_bytes: 3,
                        sha256: derivative_sha256.clone(),
                    },
                    width: Some(1),
                    height: Some(1),
                    duration_millis: None,
                },
                manifest: CompletedRequestMediaManifest {
                    id: manifest_id,
                    media_type: "image/webp".to_string(),
                    size_bytes: 3,
                    sha256: derivative_sha256.clone(),
                    chunks: vec![RequestMediaChunk {
                        index: 1,
                        object_key: output_key.to_string(),
                        plaintext_offset: 0,
                        plaintext_size_bytes: 3,
                        sha256: derivative_sha256,
                    }],
                },
            }],
            now_unix: 22,
        })
        .await
        .unwrap();
    let MediaLeaseMutation::Applied(attachment) = completed else {
        panic!("current processing lease should complete");
    };
    assert_eq!(attachment.state, RequestAttachmentState::Ready);
    assert_eq!(attachment.derivatives.len(), 1);

    let inventory = fixture
        .store
        .db
        .query_one(Statement::from_sql_and_values(
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
        "manifest_completion_preview"
    );
}

#[tokio::test]
async fn completed_manifests_reject_mutation() {
    let fixture = fixture();
    start_request(&fixture, "manifest_request", "manifest-request", 1).await;
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
    start_request(&fixture, "deleted_request", "deleted-request", 1).await;
    upload_attachment(&fixture, "deleted_request", "deleted_attachment", 10).await;

    let processing_lease = fixture
        .store
        .media()
        .claim_processing_job("processing-before-delete", 20, 30)
        .await
        .unwrap()
        .unwrap();
    let late_output_key = "media/v1/staged/deleted_attachment/late-output";
    assert_eq!(
        fixture
            .store
            .media()
            .reserve_processing_object_key(
                "deleted_attachment",
                &processing_lease.lease_token,
                processing_lease.lease_generation,
                late_output_key,
                21,
            )
            .await
            .unwrap(),
        MediaLeaseMutation::Applied(())
    );

    fixture
        .store
        .requests()
        .close_request(
            CloseRequestInput {
                request_id: "deleted_request".to_string(),
                actor_user_id: OWNER_ID.to_string(),
                actor_is_author: false,
                actor_is_maintainer: false,
                event_id: "event_delete_request".to_string(),
                now_unix: 22,
            },
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    assert!(
        fixture
            .store
            .requests()
            .request_for_tests("deleted_request")
            .await
            .unwrap()
            .is_none()
    );

    let source = ValidatedRequestAttachmentSource {
        detected_media_type: "image/png".to_string(),
        size_bytes: 4,
        sha256: SOURCE_SHA256.to_string(),
        width: Some(1),
        height: Some(1),
        duration_millis: None,
    };
    assert!(matches!(
        fixture
            .store
            .media()
            .complete_processing_job(CompleteRequestAttachmentProcessingCommand {
                attachment_id: "deleted_attachment".to_string(),
                lease_token: processing_lease.lease_token,
                lease_generation: processing_lease.lease_generation,
                source,
                derivatives: Vec::new(),
                now_unix: 23,
            })
            .await
            .unwrap(),
        MediaLeaseMutation::LeaseLost
    ));

    assert!(
        fixture
            .store
            .media()
            .claim_cleanup_job("cleanup-too-early", 89, 99)
            .await
            .unwrap()
            .is_none()
    );
    let cleanup = fixture
        .store
        .media()
        .claim_cleanup_job("cleanup-after-grace", 90, 100)
        .await
        .unwrap()
        .unwrap();
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
    let reconciled = fixture
        .store
        .media()
        .claim_cleanup_job(
            "cleanup-reconciliation",
            reconciliation_time,
            reconciliation_time + 10,
        )
        .await
        .unwrap()
        .unwrap();
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
    start_request(&fixture, "notification_request", "notification-request", 1).await;
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

    let mut listener = sqlx::postgres::PgListener::connect(&fixture.target.schema_database_url())
        .await
        .unwrap();
    listener.listen("scope_repo_changes").await.unwrap();
    let tx = fixture.store.db.begin().await.unwrap();
    replace_bindings_for_markdown(
        &tx,
        "notification_request",
        OWNER_ID,
        &scope_domain::requests::attachments::RequestAttachmentBindingTarget::Description,
        &attachment_markdown("notification_attachment"),
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
        payload["event"]["kind"]["RequestAttachmentChanged"]["request_id"],
        "notification_request"
    );
    assert_eq!(
        payload["event"]["kind"]["RequestAttachmentChanged"]["attachment_id"],
        "notification_attachment"
    );
}

#[tokio::test]
async fn expired_upload_operations_report_expiry_before_and_after_cleanup_discovery() {
    let fixture = fixture();
    start_request(&fixture, "expiry_request", "expiry-request", 1).await;
    let prepared = prepare_attachment(&fixture, "expiry_request", "expired_attachment", 10).await;
    let expires = prepared.attachment.upload_expires_at_unix;
    let command = |operation_id: &str| PrepareRequestAttachmentCommand {
        attachment_id: "replacement_attachment".to_string(),
        upload_id: "replacement_upload".to_string(),
        operation_id: operation_id.to_string(),
        request_id: "expiry_request".to_string(),
        actor_user_id: OWNER_ID.to_string(),
        target: RequestAttachmentTarget::Description,
        filename: "expired_attachment.png".to_string(),
        declared_media_type: "image/png".to_string(),
        size_bytes: 4,
        sha256: SOURCE_SHA256.to_string(),
        now_unix: expires,
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
    assert_ne!(replacement.upload_id, prepared.upload_id);
}
