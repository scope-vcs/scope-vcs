use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0042_request_media"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                SET LOCAL lock_timeout = '5s';

                CREATE TABLE scope_request_media_attachments (
                    id TEXT PRIMARY KEY,
                    repository_id TEXT NOT NULL,
                    request_id TEXT NOT NULL,
                    uploader_user_id TEXT NOT NULL,
                    upload_id TEXT NOT NULL UNIQUE,
                    operation_id TEXT NOT NULL,
                    target_json JSONB NOT NULL,
                    filename TEXT NOT NULL,
                    declared_media_type TEXT NOT NULL,
                    detected_media_type TEXT,
                    kind TEXT NOT NULL CHECK (kind IN ('Photo', 'Video')),
                    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
                    sha256 TEXT NOT NULL,
                    state TEXT NOT NULL CHECK (
                        state IN ('Prepared', 'Uploaded', 'Processing', 'Ready', 'Failed', 'Rejected')
                    ),
                    original_manifest_id TEXT,
                    original_validated_at_unix BIGINT,
                    failure_json JSONB,
                    image_width INTEGER CHECK (image_width IS NULL OR image_width > 0),
                    image_height INTEGER CHECK (image_height IS NULL OR image_height > 0),
                    video_width INTEGER CHECK (video_width IS NULL OR video_width > 0),
                    video_height INTEGER CHECK (video_height IS NULL OR video_height > 0),
                    video_duration_millis BIGINT CHECK (
                        video_duration_millis IS NULL OR video_duration_millis >= 0
                    ),
                    reserved_source_bytes BIGINT NOT NULL CHECK (reserved_source_bytes >= 0),
                    reserved_derivative_bytes BIGINT NOT NULL CHECK (reserved_derivative_bytes >= 0),
                    actual_derivative_bytes BIGINT CHECK (
                        actual_derivative_bytes IS NULL OR actual_derivative_bytes >= 0
                    ),
                    created_at_unix BIGINT NOT NULL,
                    updated_at_unix BIGINT NOT NULL,
                    upload_expires_at_unix BIGINT NOT NULL,
                    unbound_expires_at_unix BIGINT,
                    UNIQUE (request_id, uploader_user_id, operation_id)
                );
                CREATE INDEX scope_request_media_attachments_request_idx
                    ON scope_request_media_attachments (request_id, id);
                CREATE INDEX scope_request_media_attachments_repository_usage_idx
                    ON scope_request_media_attachments (repository_id, state);
                CREATE INDEX scope_request_media_attachments_expiry_idx
                    ON scope_request_media_attachments (upload_expires_at_unix, unbound_expires_at_unix);

                CREATE TABLE scope_request_media_upload_parts (
                    attachment_id TEXT NOT NULL REFERENCES scope_request_media_attachments(id),
                    upload_id TEXT NOT NULL,
                    part_number INTEGER NOT NULL CHECK (part_number > 0),
                    plaintext_size_bytes BIGINT NOT NULL CHECK (plaintext_size_bytes > 0),
                    sha256 TEXT NOT NULL,
                    object_key TEXT NOT NULL UNIQUE,
                    state TEXT NOT NULL CHECK (state IN ('Pending', 'Stored')),
                    write_token TEXT,
                    write_expires_at_unix BIGINT,
                    created_at_unix BIGINT NOT NULL,
                    stored_at_unix BIGINT,
                    PRIMARY KEY (attachment_id, part_number),
                    CHECK (
                        (state = 'Stored' AND stored_at_unix IS NOT NULL
                         AND write_token IS NULL AND write_expires_at_unix IS NULL)
                        OR (state = 'Pending' AND write_token IS NOT NULL
                            AND write_expires_at_unix IS NOT NULL)
                    )
                );

                CREATE TABLE scope_request_media_abandoned_objects (
                    object_key TEXT PRIMARY KEY,
                    attachment_id TEXT NOT NULL REFERENCES scope_request_media_attachments(id),
                    created_at_unix BIGINT NOT NULL,
                    deleted_at_unix BIGINT
                );
                CREATE INDEX scope_request_media_abandoned_objects_cleanup_idx
                    ON scope_request_media_abandoned_objects (attachment_id, deleted_at_unix);

                CREATE TABLE scope_request_media_manifests (
                    id TEXT PRIMARY KEY,
                    attachment_id TEXT NOT NULL REFERENCES scope_request_media_attachments(id),
                    derivative_id TEXT,
                    media_type TEXT NOT NULL,
                    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
                    sha256 TEXT NOT NULL,
                    completed_at_unix BIGINT NOT NULL,
                    UNIQUE (attachment_id, derivative_id)
                );

                CREATE TABLE scope_request_media_manifest_chunks (
                    manifest_id TEXT NOT NULL REFERENCES scope_request_media_manifests(id),
                    chunk_index INTEGER NOT NULL CHECK (chunk_index > 0),
                    object_key TEXT NOT NULL,
                    plaintext_offset BIGINT NOT NULL CHECK (plaintext_offset >= 0),
                    plaintext_size_bytes BIGINT NOT NULL CHECK (plaintext_size_bytes > 0),
                    sha256 TEXT NOT NULL,
                    PRIMARY KEY (manifest_id, chunk_index),
                    UNIQUE (manifest_id, object_key)
                );

                CREATE TABLE scope_request_media_derivatives (
                    id TEXT PRIMARY KEY,
                    attachment_id TEXT NOT NULL REFERENCES scope_request_media_attachments(id),
                    manifest_id TEXT NOT NULL UNIQUE REFERENCES scope_request_media_manifests(id),
                    kind TEXT NOT NULL CHECK (
                        kind IN ('ImagePreview', 'VideoPlayback', 'VideoPoster')
                    ),
                    media_type TEXT NOT NULL,
                    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
                    sha256 TEXT NOT NULL,
                    width INTEGER CHECK (width IS NULL OR width > 0),
                    height INTEGER CHECK (height IS NULL OR height > 0),
                    duration_millis BIGINT CHECK (duration_millis IS NULL OR duration_millis >= 0),
                    created_at_unix BIGINT NOT NULL,
                    UNIQUE (attachment_id, kind)
                );

                CREATE TABLE scope_request_media_bindings (
                    attachment_id TEXT NOT NULL REFERENCES scope_request_media_attachments(id),
                    request_id TEXT NOT NULL,
                    target_key TEXT NOT NULL,
                    target_kind TEXT NOT NULL CHECK (
                        target_kind IN ('Description', 'Discussion', 'Reply')
                    ),
                    discussion_id TEXT,
                    reply_id TEXT,
                    bound_at_unix BIGINT NOT NULL,
                    PRIMARY KEY (attachment_id, target_key),
                    CHECK (
                        (target_kind = 'Description' AND discussion_id IS NULL AND reply_id IS NULL)
                        OR (target_kind = 'Discussion' AND discussion_id IS NOT NULL AND reply_id IS NULL)
                        OR (target_kind = 'Reply' AND discussion_id IS NOT NULL AND reply_id IS NOT NULL)
                    )
                );
                CREATE INDEX scope_request_media_bindings_target_idx
                    ON scope_request_media_bindings (request_id, target_key, attachment_id);

                CREATE TABLE scope_request_media_processing_jobs (
                    attachment_id TEXT PRIMARY KEY REFERENCES scope_request_media_attachments(id),
                    state TEXT NOT NULL CHECK (state IN ('Queued', 'Leased', 'Completed', 'Canceled')),
                    available_at_unix BIGINT NOT NULL,
                    lease_token TEXT,
                    lease_generation BIGINT NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
                    lease_expires_at_unix BIGINT,
                    attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
                    created_at_unix BIGINT NOT NULL,
                    updated_at_unix BIGINT NOT NULL,
                    CHECK (
                        (state = 'Leased' AND lease_token IS NOT NULL AND lease_expires_at_unix IS NOT NULL)
                        OR state <> 'Leased'
                    )
                );
                CREATE INDEX scope_request_media_processing_claim_idx
                    ON scope_request_media_processing_jobs (state, available_at_unix, lease_expires_at_unix);

                CREATE TABLE scope_request_media_processing_objects (
                    object_key TEXT PRIMARY KEY,
                    attachment_id TEXT NOT NULL REFERENCES scope_request_media_attachments(id),
                    lease_generation BIGINT NOT NULL CHECK (lease_generation > 0),
                    state TEXT NOT NULL CHECK (state IN ('Pending', 'Adopted', 'Orphaned', 'Deleted')),
                    manifest_id TEXT,
                    created_at_unix BIGINT NOT NULL,
                    updated_at_unix BIGINT NOT NULL,
                    CHECK ((state = 'Adopted' AND manifest_id IS NOT NULL) OR state <> 'Adopted')
                );
                CREATE INDEX scope_request_media_processing_objects_cleanup_idx
                    ON scope_request_media_processing_objects (state, attachment_id, lease_generation);

                CREATE TABLE scope_request_media_orphan_cleanup_leases (
                    attachment_id TEXT PRIMARY KEY REFERENCES scope_request_media_attachments(id),
                    lease_token TEXT NOT NULL,
                    lease_generation BIGINT NOT NULL CHECK (lease_generation > 0),
                    lease_expires_at_unix BIGINT NOT NULL,
                    attempt INTEGER NOT NULL CHECK (attempt > 0),
                    updated_at_unix BIGINT NOT NULL
                );

                CREATE TABLE scope_request_media_cleanup_jobs (
                    attachment_id TEXT PRIMARY KEY REFERENCES scope_request_media_attachments(id),
                    repository_id TEXT NOT NULL,
                    reason TEXT NOT NULL CHECK (
                        reason IN (
                            'IncompleteUploadExpired', 'UnboundDraftExpired',
                            'RequestDeleted', 'RepositoryDeleted'
                        )
                    ),
                    state TEXT NOT NULL CHECK (state IN ('Queued', 'Leased', 'Completed')),
                    available_at_unix BIGINT NOT NULL,
                    lease_token TEXT,
                    lease_generation BIGINT NOT NULL DEFAULT 0 CHECK (lease_generation >= 0),
                    lease_expires_at_unix BIGINT,
                    attempt INTEGER NOT NULL DEFAULT 0 CHECK (attempt >= 0),
                    last_error TEXT,
                    created_at_unix BIGINT NOT NULL,
                    updated_at_unix BIGINT NOT NULL,
                    completed_at_unix BIGINT,
                    CHECK (
                        (state = 'Leased' AND lease_token IS NOT NULL AND lease_expires_at_unix IS NOT NULL)
                        OR state <> 'Leased'
                    )
                );
                CREATE INDEX scope_request_media_cleanup_claim_idx
                    ON scope_request_media_cleanup_jobs (state, available_at_unix, lease_expires_at_unix);

                CREATE TABLE scope_request_media_retry_operations (
                    attachment_id TEXT NOT NULL REFERENCES scope_request_media_attachments(id),
                    operation_id TEXT NOT NULL,
                    created_at_unix BIGINT NOT NULL,
                    PRIMARY KEY (attachment_id, operation_id)
                );

                CREATE FUNCTION scope_reject_request_media_manifest_mutation()
                RETURNS trigger LANGUAGE plpgsql AS $$
                BEGIN
                    RAISE EXCEPTION 'completed request media manifests are immutable';
                END;
                $$;
                CREATE TRIGGER scope_request_media_manifests_immutable
                    BEFORE UPDATE OR DELETE ON scope_request_media_manifests
                    FOR EACH ROW EXECUTE FUNCTION scope_reject_request_media_manifest_mutation();
                CREATE TRIGGER scope_request_media_manifest_chunks_immutable
                    BEFORE UPDATE OR DELETE ON scope_request_media_manifest_chunks
                    FOR EACH ROW EXECUTE FUNCTION scope_reject_request_media_manifest_mutation();
                "#,
            )
            .await?;
        Ok(())
    }
}
