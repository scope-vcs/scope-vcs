-- Schema at 578bec00da088598919082b35a7153f62bf0b860, through m0042_request_media.
-- Captured from the original chain; later migrations remain separate.
-- Extension creation is shared by every schema in this database.
SELECT pg_advisory_xact_lock(hashtextextended('scope:metadata-extension:pg_trgm', 0));
CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;

CREATE FUNCTION scope_check_repository_workflow_catalog_rejection() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
                BEGIN
                    IF NEW.configuration_error IS NOT NULL AND EXISTS (
                        SELECT 1
                        FROM scope_repository_workflow_files
                        WHERE repo_id = NEW.repo_id
                    ) THEN
                        RAISE EXCEPTION 'rejected repository workflow catalog cannot contain files';
                    END IF;

                    RETURN NEW;
                END;
                $$;

CREATE FUNCTION scope_check_repository_workflow_file() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
                BEGIN
                    IF EXISTS (
                        SELECT 1
                        FROM scope_repository_workflow_catalogs
                        WHERE repo_id = NEW.repo_id
                          AND configuration_error IS NOT NULL
                    ) THEN
                        RAISE EXCEPTION 'rejected repository workflow catalog cannot contain files';
                    END IF;

                    IF TG_OP = 'INSERT' OR
                       NEW.repo_id <> OLD.repo_id OR
                       NEW.path <> OLD.path THEN
                        IF (
                            SELECT count(*)
                            FROM scope_repository_workflow_files
                            WHERE repo_id = NEW.repo_id
                        ) >= 64 THEN
                            RAISE EXCEPTION 'repository workflow catalog cannot contain more than 64 files';
                        END IF;
                    END IF;

                    RETURN NEW;
                END;
                $$;

CREATE FUNCTION scope_reject_request_media_manifest_mutation() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
                BEGIN
                    RAISE EXCEPTION 'completed request media manifests are immutable';
                END;
                $$;

CREATE TABLE scope_auth_identities (
    provider character varying NOT NULL,
    subject character varying NOT NULL,
    user_id character varying NOT NULL
);

CREATE TABLE scope_cache_deletion_queue (
    repository_id text NOT NULL,
    checksum_sha256 character varying(64) NOT NULL,
    not_before_unix bigint NOT NULL,
    attempts integer NOT NULL,
    last_error text,
    CONSTRAINT scope_cache_deletion_queue_values CHECK ((((checksum_sha256)::text ~ '^[0-9a-f]{64}$'::text) AND (not_before_unix >= 0) AND (attempts >= 0) AND ((last_error IS NULL) OR ((char_length(last_error) >= 1) AND (char_length(last_error) <= 8192)))))
);

CREATE TABLE scope_cache_objects (
    repository_id text NOT NULL,
    checksum_sha256 character varying(64) NOT NULL,
    storage_backend character varying(64) NOT NULL,
    object_key text NOT NULL,
    size_bytes bigint NOT NULL,
    created_at_unix bigint NOT NULL,
    last_accessed_at_unix bigint NOT NULL,
    CONSTRAINT scope_cache_objects_values CHECK ((((checksum_sha256)::text ~ '^[0-9a-f]{64}$'::text) AND ((char_length((storage_backend)::text) >= 1) AND (char_length((storage_backend)::text) <= 64)) AND ((storage_backend)::text ~ '^[a-z0-9]+(-[a-z0-9]+)*$'::text) AND (object_key = ((('repos/'::text || repository_id) || '/objects/sha256/'::text) || (checksum_sha256)::text)) AND ((size_bytes >= 1) AND (size_bytes <= 1073741824)) AND (created_at_unix >= 0) AND (last_accessed_at_unix >= created_at_unix)))
);

CREATE TABLE scope_cache_orphan_uploads (
    object_key text NOT NULL,
    repository_id text NOT NULL,
    not_before_unix bigint NOT NULL,
    attempts integer NOT NULL,
    last_error text,
    CONSTRAINT scope_cache_orphan_uploads_values CHECK (((object_key = ((('repos/'::text || repository_id) || '/objects/sha256/'::text) || "right"(object_key, 64))) AND ("right"(object_key, 64) ~ '^[0-9a-f]{64}$'::text) AND (not_before_unix >= 0) AND (attempts >= 0) AND ((last_error IS NULL) OR ((char_length(last_error) >= 1) AND (char_length(last_error) <= 8192)))))
);

CREATE TABLE scope_cache_references (
    repository_id text NOT NULL,
    identity_digest character varying(64) NOT NULL,
    compatibility_group_digest character varying(64) NOT NULL,
    checksum_sha256 character varying(64) NOT NULL,
    created_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    last_accessed_at_unix bigint NOT NULL,
    CONSTRAINT scope_cache_references_values CHECK ((((identity_digest)::text ~ '^[0-9a-f]{64}$'::text) AND ((compatibility_group_digest)::text ~ '^[0-9a-f]{64}$'::text) AND ((checksum_sha256)::text ~ '^[0-9a-f]{64}$'::text) AND (created_at_unix >= 0) AND (last_accessed_at_unix >= created_at_unix) AND (expires_at_unix > last_accessed_at_unix)))
);

CREATE TABLE scope_cache_uploads (
    upload_id text NOT NULL,
    repository_id text NOT NULL,
    identity_digest character varying(64) NOT NULL,
    compatibility_group_digest character varying(64) NOT NULL,
    checksum_sha256 character varying(64) NOT NULL,
    storage_backend character varying(64) NOT NULL,
    object_key text NOT NULL,
    size_bytes bigint NOT NULL,
    state text NOT NULL,
    created_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    CONSTRAINT scope_cache_uploads_values CHECK ((((char_length(upload_id) >= 1) AND (char_length(upload_id) <= 128)) AND (upload_id !~ '[[:space:]]'::text) AND ((identity_digest)::text ~ '^[0-9a-f]{64}$'::text) AND ((compatibility_group_digest)::text ~ '^[0-9a-f]{64}$'::text) AND ((checksum_sha256)::text ~ '^[0-9a-f]{64}$'::text) AND ((char_length((storage_backend)::text) >= 1) AND (char_length((storage_backend)::text) <= 64)) AND ((storage_backend)::text ~ '^[a-z0-9]+(-[a-z0-9]+)*$'::text) AND (object_key = ((('repos/'::text || repository_id) || '/objects/sha256/'::text) || (checksum_sha256)::text)) AND ((size_bytes >= 1) AND (size_bytes <= 1073741824)) AND (state = ANY (ARRAY['active'::text, 'deleting'::text, 'committed'::text])) AND (created_at_unix >= 0) AND (expires_at_unix > created_at_unix) AND (expires_at_unix <= (created_at_unix + 1800))))
);

CREATE TABLE scope_cli_browser_logins (
    request_id character varying NOT NULL,
    request_secret_hash character varying NOT NULL,
    callback_url text NOT NULL,
    callback_code_hash character varying,
    created_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    completed_user_id character varying,
    completed_at_unix bigint,
    consumed_at_unix bigint
);

CREATE TABLE scope_cli_device_logins (
    device_code_hash character varying NOT NULL,
    user_code_hash character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    completed_user_id character varying,
    completed_at_unix bigint,
    consumed_at_unix bigint
);

CREATE TABLE scope_cli_exchange_grants (
    grant_hash character varying NOT NULL,
    user_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    consumed_at_unix bigint
);

CREATE TABLE scope_cli_sessions (
    id character varying NOT NULL,
    token_hash character varying NOT NULL,
    user_id character varying NOT NULL,
    label character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    last_used_at_unix bigint,
    expires_at_unix bigint NOT NULL,
    revoked_at_unix bigint
);

CREATE TABLE scope_file_changes (
    repo_id character varying NOT NULL,
    commit_id character varying NOT NULL,
    ordinal bigint NOT NULL,
    path text NOT NULL,
    old_content jsonb,
    new_content jsonb,
    visibility character varying NOT NULL,
    CONSTRAINT scope_file_change_ordinal CHECK ((ordinal >= 0))
);

CREATE TABLE scope_git_compaction_jobs (
    repo_id text NOT NULL,
    target_sequence bigint NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    next_run_at_unix bigint NOT NULL,
    lease_generation text,
    lease_owner text,
    lease_expires_at_unix bigint,
    last_error text,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_git_compaction_job_values CHECK (((target_sequence > 0) AND (attempts >= 0) AND (next_run_at_unix >= 0) AND ((lease_generation IS NULL) = (lease_owner IS NULL)) AND ((lease_generation IS NULL) = (lease_expires_at_unix IS NULL)) AND ((lease_expires_at_unix IS NULL) OR (lease_expires_at_unix >= 0)) AND ((last_error IS NULL) OR ((char_length(last_error) >= 1) AND (char_length(last_error) <= 2000))) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix)))
);

CREATE TABLE scope_git_heads (
    repo_id character varying NOT NULL,
    head_oid character varying NOT NULL,
    push_sequence bigint NOT NULL,
    change_version bigint NOT NULL,
    manifest_object_key character varying NOT NULL,
    manifest_sha256 character varying NOT NULL,
    manifest_size_bytes bigint NOT NULL,
    CONSTRAINT scope_git_head_values CHECK (((push_sequence >= 0) AND (change_version >= 0) AND (manifest_size_bytes >= 0)))
);

CREATE TABLE scope_git_segment_references (
    segment_id text NOT NULL,
    ref_kind text NOT NULL,
    ref_id text NOT NULL,
    CONSTRAINT scope_git_segment_reference_values CHECK (((ref_kind = ANY (ARRAY['push_trigger_source'::text, 'run_source'::text])) AND (length(btrim(ref_id)) > 0)))
);

CREATE TABLE scope_git_segment_uploads (
    segment_id text NOT NULL,
    repo_id text NOT NULL,
    object_key text NOT NULL,
    state text NOT NULL,
    sha256 text,
    plaintext_bytes bigint,
    encrypted_bytes bigint,
    encoding_version integer NOT NULL,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_git_segment_upload_state CHECK ((state = ANY (ARRAY['uploading'::text, 'ready'::text, 'published'::text, 'retained'::text, 'deleting'::text, 'deleted'::text]))),
    CONSTRAINT scope_git_segment_upload_values CHECK (((length(btrim(segment_id)) > 0) AND (length(btrim(object_key)) > 0) AND (encoding_version > 0) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix) AND ((sha256 IS NULL) OR (length(sha256) = 64)) AND ((plaintext_bytes IS NULL) OR (plaintext_bytes >= 0)) AND ((encrypted_bytes IS NULL) OR (encrypted_bytes >= 0)) AND ((state <> ALL (ARRAY['ready'::text, 'published'::text, 'retained'::text])) OR ((sha256 IS NOT NULL) AND (plaintext_bytes IS NOT NULL) AND (encrypted_bytes IS NOT NULL)))))
);

CREATE TABLE scope_git_segments (
    repo_id character varying NOT NULL,
    first_sequence bigint NOT NULL,
    base_oid character varying,
    head_oid character varying NOT NULL,
    last_sequence bigint NOT NULL,
    geometric_tier integer NOT NULL,
    segment_id text NOT NULL
);

CREATE TABLE scope_live_files (
    repo_id character varying NOT NULL,
    path text NOT NULL,
    content jsonb NOT NULL
);

CREATE TABLE scope_logical_commits (
    id character varying NOT NULL,
    repo_id character varying NOT NULL,
    ordinal bigint NOT NULL,
    origin jsonb NOT NULL,
    author_id character varying NOT NULL,
    message text NOT NULL,
    occurred_at_unix bigint,
    CONSTRAINT scope_logical_commit_ordinal CHECK ((ordinal >= 0))
);

CREATE TABLE scope_metadata_locks (
    key character varying NOT NULL
);

CREATE TABLE scope_object_references (
    object_key character varying NOT NULL,
    ref_kind character varying NOT NULL,
    ref_id character varying NOT NULL
);

CREATE TABLE scope_orphan_object_jobs (
    object_key character varying NOT NULL,
    generation character varying NOT NULL,
    sha256 character varying NOT NULL,
    git_oid character varying NOT NULL,
    size_bytes bigint NOT NULL,
    attempts integer NOT NULL,
    next_run_at_unix bigint NOT NULL,
    last_error text,
    completed_at_unix bigint,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_blob_cleanup_values CHECK (((size_bytes >= 0) AND (attempts >= 0) AND (next_run_at_unix >= 0) AND (created_at_unix >= 0) AND (updated_at_unix >= 0) AND ((completed_at_unix IS NULL) OR (completed_at_unix >= 0))))
);

CREATE TABLE scope_outbox_jobs (
    id character varying NOT NULL,
    idempotency_key character varying NOT NULL,
    kind character varying NOT NULL,
    repo_id character varying NOT NULL,
    repo_version bigint NOT NULL,
    payload jsonb NOT NULL,
    state character varying NOT NULL,
    attempts bigint NOT NULL,
    next_run_at_unix bigint NOT NULL,
    lease_owner character varying,
    lease_expires_at_unix bigint,
    last_error text,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    completed_at_unix bigint,
    CONSTRAINT scope_outbox_jobs_push_workflow_schema_v5 CHECK ((((kind)::text <> 'push_main_trigger_evaluation'::text) OR (completed_at_unix IS NOT NULL) OR (payload @> '{"workflow_schema_version": 5}'::jsonb))),
    CONSTRAINT scope_outbox_values CHECK (((repo_version >= 0) AND (attempts >= 0) AND (next_run_at_unix >= 0) AND (created_at_unix >= 0) AND (updated_at_unix >= 0) AND ((state)::text = ANY ((ARRAY['ready'::character varying, 'running'::character varying, 'succeeded'::character varying, 'failed'::character varying])::text[])) AND ((lease_expires_at_unix IS NULL) OR (lease_expires_at_unix >= 0)) AND ((completed_at_unix IS NULL) OR (completed_at_unix >= 0))))
);

CREATE TABLE scope_projection_files (
    repo_id character varying NOT NULL,
    repo_version bigint NOT NULL,
    source character varying NOT NULL,
    audience character varying NOT NULL,
    path_key character varying NOT NULL,
    path character varying NOT NULL,
    oid character varying NOT NULL,
    visibility character varying NOT NULL,
    object_key character varying NOT NULL,
    sha256 character varying NOT NULL,
    size_bytes bigint NOT NULL,
    git_file_mode character varying NOT NULL,
    CONSTRAINT scope_projection_file_values CHECK (((repo_version >= 0) AND ((source)::text = 'live'::text) AND ((audience)::text = ANY ((ARRAY['private'::character varying, 'public'::character varying])::text[])) AND (size_bytes >= 0) AND ((git_file_mode)::text = ANY ((ARRAY['100644'::character varying, '100755'::character varying])::text[]))))
);

CREATE TABLE scope_projection_read_models (
    repo_id character varying NOT NULL,
    repo_version bigint NOT NULL,
    source character varying NOT NULL,
    audience character varying NOT NULL,
    rebuilt_at_unix bigint NOT NULL,
    file_count bigint NOT NULL,
    head_oid character varying,
    identity_version smallint NOT NULL,
    CONSTRAINT scope_projection_read_model_identity CHECK (((identity_version = 2) AND ((head_oid IS NULL) OR ((char_length((head_oid)::text) = 40) AND ((head_oid)::text ~ '^[0-9A-Fa-f]+$'::text))))),
    CONSTRAINT scope_projection_read_model_values CHECK (((repo_version >= 0) AND (rebuilt_at_unix >= 0) AND (file_count >= 0) AND ((source)::text = 'live'::text) AND ((audience)::text = ANY ((ARRAY['private'::character varying, 'public'::character varying])::text[]))))
);

CREATE TABLE scope_push_trigger_evaluations (
    repo_id character varying NOT NULL,
    change_version bigint NOT NULL,
    head_oid character varying NOT NULL,
    state character varying NOT NULL,
    message text,
    checks jsonb NOT NULL,
    created_at_unix bigint NOT NULL,
    completed_at_unix bigint,
    CONSTRAINT scope_push_trigger_evaluation_values CHECK (((change_version > 0) AND (length((head_oid)::text) = 40) AND (created_at_unix >= 0) AND ((state)::text = ANY ((ARRAY['pending'::character varying, 'succeeded'::character varying, 'configuration-error'::character varying, 'failed'::character varying])::text[])) AND ((((state)::text = 'pending'::text) AND (message IS NULL) AND (completed_at_unix IS NULL)) OR (((state)::text = 'succeeded'::text) AND (message IS NULL) AND (completed_at_unix IS NOT NULL)) OR (((state)::text = ANY ((ARRAY['configuration-error'::character varying, 'failed'::character varying])::text[])) AND (length(btrim(message)) > 0) AND (completed_at_unix IS NOT NULL)))))
);

CREATE TABLE scope_repo_storage_cleanup_jobs (
    repo_id character varying NOT NULL,
    generation character varying NOT NULL,
    owner_handle character varying NOT NULL,
    repo_name character varying NOT NULL,
    attempts integer NOT NULL,
    next_run_at_unix bigint NOT NULL,
    last_error text,
    completed_at_unix bigint,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    incarnation_id text NOT NULL,
    CONSTRAINT scope_repo_cleanup_incarnation_nonempty CHECK ((length(btrim(incarnation_id)) > 0)),
    CONSTRAINT scope_repo_cleanup_values CHECK (((attempts >= 0) AND (next_run_at_unix >= 0) AND (created_at_unix >= 0) AND (updated_at_unix >= 0) AND ((completed_at_unix IS NULL) OR (completed_at_unix >= 0))))
);

CREATE TABLE scope_repositories (
    id character varying NOT NULL,
    owner_handle character varying NOT NULL,
    name character varying NOT NULL,
    owner_user_id character varying NOT NULL,
    publication_state character varying NOT NULL,
    change_version bigint NOT NULL,
    repo_config jsonb NOT NULL,
    policy jsonb NOT NULL,
    incarnation_id text NOT NULL,
    description text,
    website_url text,
    CONSTRAINT scope_repositories_nonnegative_version CHECK ((change_version >= 0)),
    CONSTRAINT scope_repository_incarnation_nonempty CHECK ((length(btrim(incarnation_id)) > 0))
);

CREATE TABLE scope_repository_first_push_tokens (
    repo_id character varying NOT NULL,
    token_hash character varying NOT NULL,
    owner_user_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    used_at_unix bigint,
    CONSTRAINT scope_first_push_token_times CHECK (((created_at_unix >= 0) AND (expires_at_unix >= 0) AND ((used_at_unix IS NULL) OR (used_at_unix >= 0))))
);

CREATE TABLE scope_repository_git_push_tokens (
    repo_id character varying NOT NULL,
    token_hash character varying NOT NULL,
    owner_user_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    CONSTRAINT scope_git_push_token_time CHECK ((created_at_unix >= 0))
);

CREATE TABLE scope_repository_history_entries (
    repo_id text NOT NULL,
    audience text NOT NULL,
    "position" bigint NOT NULL,
    source_id text NOT NULL,
    payload jsonb NOT NULL,
    CONSTRAINT scope_repository_history_entries_position_check CHECK (("position" >= 0))
);

CREATE TABLE scope_repository_history_views (
    repo_id text NOT NULL,
    audience text NOT NULL,
    repo_version bigint NOT NULL,
    generation text NOT NULL,
    identity_version smallint NOT NULL,
    available boolean NOT NULL,
    visible_files boolean NOT NULL,
    head_oid text,
    history_version text NOT NULL,
    CONSTRAINT scope_repository_history_views_audience_check CHECK ((audience = ANY (ARRAY['private'::text, 'public'::text]))),
    CONSTRAINT scope_repository_history_views_repo_version_check CHECK ((repo_version >= 0))
);

CREATE TABLE scope_repository_invites (
    id character varying NOT NULL,
    repo_id character varying NOT NULL,
    invited_email character varying NOT NULL,
    invited_email_normalized character varying NOT NULL,
    permissions jsonb NOT NULL,
    invited_by_user_id character varying NOT NULL,
    state character varying NOT NULL,
    token_hash character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    expires_at_unix bigint NOT NULL,
    accepted_by_user_id character varying,
    accepted_at_unix bigint,
    revoked_at_unix bigint,
    CONSTRAINT scope_repository_invite_times CHECK (((created_at_unix >= 0) AND (updated_at_unix >= 0) AND (expires_at_unix >= 0) AND ((accepted_at_unix IS NULL) OR (accepted_at_unix >= 0)) AND ((revoked_at_unix IS NULL) OR (revoked_at_unix >= 0))))
);

CREATE TABLE scope_repository_landing_files (
    repo_id text NOT NULL,
    path text NOT NULL,
    oid text NOT NULL,
    sha256 text NOT NULL,
    size_bytes bigint NOT NULL,
    git_file_mode text NOT NULL,
    content_bytes bytea NOT NULL,
    CONSTRAINT scope_repository_landing_file_values CHECK (((path = '/README.html'::text) AND ((char_length(oid) >= 1) AND (char_length(oid) <= 128)) AND (char_length(sha256) = 64) AND (sha256 = lower(sha256)) AND (sha256 ~ '^[0-9a-f]{64}$'::text) AND ((size_bytes >= 0) AND (size_bytes <= 1048576)) AND (octet_length(content_bytes) = size_bytes) AND (git_file_mode = ANY (ARRAY['100644'::text, '100755'::text]))))
);

CREATE TABLE scope_repository_members (
    repo_id character varying NOT NULL,
    user_id character varying NOT NULL,
    permissions jsonb NOT NULL,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_repository_member_times CHECK (((created_at_unix >= 0) AND (updated_at_unix >= 0)))
);

CREATE TABLE scope_repository_workflow_catalogs (
    repo_id text NOT NULL,
    source_head_oid text NOT NULL,
    source_change_version bigint NOT NULL,
    configuration_error text,
    CONSTRAINT scope_repository_workflow_catalog_values CHECK (((char_length(source_head_oid) = 40) AND (source_head_oid = lower(source_head_oid)) AND (source_head_oid ~ '^[0-9a-f]{40}$'::text) AND (source_change_version > 0) AND ((configuration_error IS NULL) OR ((octet_length(configuration_error) >= 1) AND (octet_length(configuration_error) <= 4096)))))
);

CREATE TABLE scope_repository_workflow_files (
    repo_id text NOT NULL,
    path text NOT NULL,
    oid text NOT NULL,
    size_bytes bigint NOT NULL,
    git_file_mode text NOT NULL,
    content_bytes bytea NOT NULL,
    CONSTRAINT scope_repository_workflow_file_values CHECK (((path ~ '^/\.scope/runs/[a-z0-9]+(-[a-z0-9]+)*\.ya?ml$'::text) AND ((octet_length(regexp_replace(regexp_replace(path, '^/\.scope/runs/'::text, ''::text), '\.ya?ml$'::text, ''::text)) >= 1) AND (octet_length(regexp_replace(regexp_replace(path, '^/\.scope/runs/'::text, ''::text), '\.ya?ml$'::text, ''::text)) <= 64)) AND (char_length(oid) = 40) AND (oid = lower(oid)) AND (oid ~ '^[0-9a-f]{40}$'::text) AND ((size_bytes >= 0) AND (size_bytes <= 65536)) AND (octet_length(content_bytes) = size_bytes) AND (git_file_mode = ANY (ARRAY['100644'::text, '100755'::text]))))
);

CREATE TABLE scope_request_discussion_read_states (
    discussion_id character varying NOT NULL,
    user_id character varying NOT NULL,
    read_through_position bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_discussion_read_values CHECK (((read_through_position >= 0) AND (updated_at_unix >= 0)))
);

CREATE TABLE scope_request_discussion_replies (
    id character varying NOT NULL,
    discussion_id character varying NOT NULL,
    "position" bigint NOT NULL,
    author_user_id character varying NOT NULL,
    body_markdown text NOT NULL,
    reply_to_reply_id character varying,
    client_reply_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_discussion_reply_values CHECK ((("position" > 0) AND (length(btrim(body_markdown)) > 0) AND (created_at_unix >= 0)))
);

CREATE TABLE scope_request_discussions (
    id character varying NOT NULL,
    request_id character varying NOT NULL,
    opened_position bigint NOT NULL,
    last_activity_position bigint NOT NULL,
    author_user_id character varying NOT NULL,
    body_markdown text NOT NULL,
    status character varying NOT NULL,
    client_discussion_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    resolved_at_unix bigint,
    resolved_by_user_id character varying,
    revision_id character varying,
    commit_oid character varying,
    path text,
    CONSTRAINT scope_request_discussion_values CHECK (((opened_position > 0) AND (last_activity_position >= opened_position) AND ((status)::text = ANY ((ARRAY['Open'::character varying, 'Resolved'::character varying])::text[])) AND (length(btrim(body_markdown)) > 0) AND (created_at_unix >= 0) AND ((resolved_at_unix IS NULL) OR (resolved_at_unix >= 0)) AND ((commit_oid IS NULL) OR (revision_id IS NOT NULL)) AND ((path IS NULL) OR (commit_oid IS NOT NULL))))
);

CREATE TABLE scope_request_events (
    id character varying NOT NULL,
    request_id character varying NOT NULL,
    actor_user_id character varying NOT NULL,
    kind character varying NOT NULL,
    "position" bigint NOT NULL,
    payload jsonb NOT NULL,
    created_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_event_values CHECK ((("position" > 0) AND (created_at_unix >= 0)))
);

CREATE TABLE scope_request_invitees (
    request_id character varying NOT NULL,
    user_id character varying NOT NULL,
    invited_by_user_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_invitee_values CHECK ((created_at_unix >= 0))
);

CREATE TABLE scope_request_media_abandoned_objects (
    object_key text NOT NULL,
    attachment_id text NOT NULL,
    created_at_unix bigint NOT NULL,
    deleted_at_unix bigint
);

CREATE TABLE scope_request_media_attachments (
    id text NOT NULL,
    repository_id text NOT NULL,
    request_id text NOT NULL,
    uploader_user_id text NOT NULL,
    upload_id text NOT NULL,
    operation_id text NOT NULL,
    target_json jsonb NOT NULL,
    filename text NOT NULL,
    declared_media_type text NOT NULL,
    detected_media_type text,
    kind text NOT NULL,
    size_bytes bigint NOT NULL,
    sha256 text NOT NULL,
    state text NOT NULL,
    original_manifest_id text,
    original_validated_at_unix bigint,
    failure_json jsonb,
    image_width integer,
    image_height integer,
    video_width integer,
    video_height integer,
    video_duration_millis bigint,
    reserved_source_bytes bigint NOT NULL,
    reserved_derivative_bytes bigint NOT NULL,
    actual_derivative_bytes bigint,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    upload_expires_at_unix bigint NOT NULL,
    unbound_expires_at_unix bigint,
    CONSTRAINT scope_request_media_attachments_actual_derivative_bytes_check CHECK (((actual_derivative_bytes IS NULL) OR (actual_derivative_bytes >= 0))),
    CONSTRAINT scope_request_media_attachments_image_height_check CHECK (((image_height IS NULL) OR (image_height > 0))),
    CONSTRAINT scope_request_media_attachments_image_width_check CHECK (((image_width IS NULL) OR (image_width > 0))),
    CONSTRAINT scope_request_media_attachments_kind_check CHECK ((kind = ANY (ARRAY['Photo'::text, 'Video'::text]))),
    CONSTRAINT scope_request_media_attachments_reserved_derivative_bytes_check CHECK ((reserved_derivative_bytes >= 0)),
    CONSTRAINT scope_request_media_attachments_reserved_source_bytes_check CHECK ((reserved_source_bytes >= 0)),
    CONSTRAINT scope_request_media_attachments_size_bytes_check CHECK ((size_bytes >= 0)),
    CONSTRAINT scope_request_media_attachments_state_check CHECK ((state = ANY (ARRAY['Prepared'::text, 'Uploaded'::text, 'Processing'::text, 'Ready'::text, 'Failed'::text, 'Rejected'::text]))),
    CONSTRAINT scope_request_media_attachments_video_duration_millis_check CHECK (((video_duration_millis IS NULL) OR (video_duration_millis >= 0))),
    CONSTRAINT scope_request_media_attachments_video_height_check CHECK (((video_height IS NULL) OR (video_height > 0))),
    CONSTRAINT scope_request_media_attachments_video_width_check CHECK (((video_width IS NULL) OR (video_width > 0)))
);

CREATE TABLE scope_request_media_bindings (
    attachment_id text NOT NULL,
    request_id text NOT NULL,
    target_key text NOT NULL,
    target_kind text NOT NULL,
    discussion_id text,
    reply_id text,
    bound_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_media_bindings_check CHECK ((((target_kind = 'Description'::text) AND (discussion_id IS NULL) AND (reply_id IS NULL)) OR ((target_kind = 'Discussion'::text) AND (discussion_id IS NOT NULL) AND (reply_id IS NULL)) OR ((target_kind = 'Reply'::text) AND (discussion_id IS NOT NULL) AND (reply_id IS NOT NULL)))),
    CONSTRAINT scope_request_media_bindings_target_kind_check CHECK ((target_kind = ANY (ARRAY['Description'::text, 'Discussion'::text, 'Reply'::text])))
);

CREATE TABLE scope_request_media_cleanup_jobs (
    attachment_id text NOT NULL,
    repository_id text NOT NULL,
    reason text NOT NULL,
    state text NOT NULL,
    available_at_unix bigint NOT NULL,
    lease_token text,
    lease_generation bigint DEFAULT 0 NOT NULL,
    lease_expires_at_unix bigint,
    attempt integer DEFAULT 0 NOT NULL,
    last_error text,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    completed_at_unix bigint,
    CONSTRAINT scope_request_media_cleanup_jobs_attempt_check CHECK ((attempt >= 0)),
    CONSTRAINT scope_request_media_cleanup_jobs_check CHECK ((((state = 'Leased'::text) AND (lease_token IS NOT NULL) AND (lease_expires_at_unix IS NOT NULL)) OR (state <> 'Leased'::text))),
    CONSTRAINT scope_request_media_cleanup_jobs_lease_generation_check CHECK ((lease_generation >= 0)),
    CONSTRAINT scope_request_media_cleanup_jobs_reason_check CHECK ((reason = ANY (ARRAY['IncompleteUploadExpired'::text, 'UnboundDraftExpired'::text, 'RequestDeleted'::text, 'RepositoryDeleted'::text]))),
    CONSTRAINT scope_request_media_cleanup_jobs_state_check CHECK ((state = ANY (ARRAY['Queued'::text, 'Leased'::text, 'Completed'::text])))
);

CREATE TABLE scope_request_media_derivatives (
    id text NOT NULL,
    attachment_id text NOT NULL,
    manifest_id text NOT NULL,
    kind text NOT NULL,
    media_type text NOT NULL,
    size_bytes bigint NOT NULL,
    sha256 text NOT NULL,
    width integer,
    height integer,
    duration_millis bigint,
    created_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_media_derivatives_duration_millis_check CHECK (((duration_millis IS NULL) OR (duration_millis >= 0))),
    CONSTRAINT scope_request_media_derivatives_height_check CHECK (((height IS NULL) OR (height > 0))),
    CONSTRAINT scope_request_media_derivatives_kind_check CHECK ((kind = ANY (ARRAY['ImagePreview'::text, 'VideoPlayback'::text, 'VideoPoster'::text]))),
    CONSTRAINT scope_request_media_derivatives_size_bytes_check CHECK ((size_bytes >= 0)),
    CONSTRAINT scope_request_media_derivatives_width_check CHECK (((width IS NULL) OR (width > 0)))
);

CREATE TABLE scope_request_media_manifest_chunks (
    manifest_id text NOT NULL,
    chunk_index integer NOT NULL,
    object_key text NOT NULL,
    plaintext_offset bigint NOT NULL,
    plaintext_size_bytes bigint NOT NULL,
    sha256 text NOT NULL,
    CONSTRAINT scope_request_media_manifest_chunks_chunk_index_check CHECK ((chunk_index > 0)),
    CONSTRAINT scope_request_media_manifest_chunks_plaintext_offset_check CHECK ((plaintext_offset >= 0)),
    CONSTRAINT scope_request_media_manifest_chunks_plaintext_size_bytes_check CHECK ((plaintext_size_bytes > 0))
);

CREATE TABLE scope_request_media_manifests (
    id text NOT NULL,
    attachment_id text NOT NULL,
    derivative_id text,
    media_type text NOT NULL,
    size_bytes bigint NOT NULL,
    sha256 text NOT NULL,
    completed_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_media_manifests_size_bytes_check CHECK ((size_bytes >= 0))
);

CREATE TABLE scope_request_media_orphan_cleanup_leases (
    attachment_id text NOT NULL,
    lease_token text NOT NULL,
    lease_generation bigint NOT NULL,
    lease_expires_at_unix bigint NOT NULL,
    attempt integer NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_media_orphan_cleanup_lease_lease_generation_check CHECK ((lease_generation > 0)),
    CONSTRAINT scope_request_media_orphan_cleanup_leases_attempt_check CHECK ((attempt > 0))
);

CREATE TABLE scope_request_media_processing_jobs (
    attachment_id text NOT NULL,
    state text NOT NULL,
    available_at_unix bigint NOT NULL,
    lease_token text,
    lease_generation bigint DEFAULT 0 NOT NULL,
    lease_expires_at_unix bigint,
    attempt integer DEFAULT 0 NOT NULL,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_media_processing_jobs_attempt_check CHECK ((attempt >= 0)),
    CONSTRAINT scope_request_media_processing_jobs_check CHECK ((((state = 'Leased'::text) AND (lease_token IS NOT NULL) AND (lease_expires_at_unix IS NOT NULL)) OR (state <> 'Leased'::text))),
    CONSTRAINT scope_request_media_processing_jobs_lease_generation_check CHECK ((lease_generation >= 0)),
    CONSTRAINT scope_request_media_processing_jobs_state_check CHECK ((state = ANY (ARRAY['Queued'::text, 'Leased'::text, 'Completed'::text, 'Canceled'::text])))
);

CREATE TABLE scope_request_media_processing_objects (
    object_key text NOT NULL,
    attachment_id text NOT NULL,
    lease_generation bigint NOT NULL,
    state text NOT NULL,
    manifest_id text,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_media_processing_objects_check CHECK ((((state = 'Adopted'::text) AND (manifest_id IS NOT NULL)) OR (state <> 'Adopted'::text))),
    CONSTRAINT scope_request_media_processing_objects_lease_generation_check CHECK ((lease_generation > 0)),
    CONSTRAINT scope_request_media_processing_objects_state_check CHECK ((state = ANY (ARRAY['Pending'::text, 'Adopted'::text, 'Orphaned'::text, 'Deleted'::text])))
);

CREATE TABLE scope_request_media_retry_operations (
    attachment_id text NOT NULL,
    operation_id text NOT NULL,
    created_at_unix bigint NOT NULL
);

CREATE TABLE scope_request_media_upload_parts (
    attachment_id text NOT NULL,
    upload_id text NOT NULL,
    part_number integer NOT NULL,
    plaintext_size_bytes bigint NOT NULL,
    sha256 text NOT NULL,
    object_key text NOT NULL,
    state text NOT NULL,
    write_token text,
    write_expires_at_unix bigint,
    created_at_unix bigint NOT NULL,
    stored_at_unix bigint,
    CONSTRAINT scope_request_media_upload_parts_check CHECK ((((state = 'Stored'::text) AND (stored_at_unix IS NOT NULL) AND (write_token IS NULL) AND (write_expires_at_unix IS NULL)) OR ((state = 'Pending'::text) AND (write_token IS NOT NULL) AND (write_expires_at_unix IS NOT NULL)))),
    CONSTRAINT scope_request_media_upload_parts_part_number_check CHECK ((part_number > 0)),
    CONSTRAINT scope_request_media_upload_parts_plaintext_size_bytes_check CHECK ((plaintext_size_bytes > 0)),
    CONSTRAINT scope_request_media_upload_parts_state_check CHECK ((state = ANY (ARRAY['Pending'::text, 'Stored'::text])))
);

CREATE TABLE scope_request_ratings (
    id character varying NOT NULL,
    request_id character varying NOT NULL,
    rater_user_id character varying NOT NULL,
    subject_user_id character varying NOT NULL,
    score integer NOT NULL,
    reason text NOT NULL,
    created_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_rating_participants_distinct CHECK (((rater_user_id)::text <> (subject_user_id)::text)),
    CONSTRAINT scope_request_rating_reason CHECK (((reason = btrim(reason)) AND ((octet_length(reason) >= 1) AND (octet_length(reason) <= 1024)))),
    CONSTRAINT scope_request_rating_score CHECK (((score >= 1) AND (score <= 5))),
    CONSTRAINT scope_request_rating_time CHECK ((created_at_unix >= 0))
);

CREATE TABLE scope_request_revisions (
    id character varying NOT NULL,
    request_id character varying NOT NULL,
    "position" bigint NOT NULL,
    actor_user_id character varying NOT NULL,
    old_head_oid character varying NOT NULL,
    new_head_oid character varying NOT NULL,
    git_snapshot jsonb NOT NULL,
    created_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_revision_values CHECK ((("position" > 0) AND (created_at_unix >= 0) AND (length((old_head_oid)::text) > 0) AND (length((new_head_oid)::text) > 0)))
);

CREATE TABLE scope_requests (
    id character varying NOT NULL,
    repo_id character varying NOT NULL,
    name character varying NOT NULL,
    author_user_id character varying NOT NULL,
    author_role character varying NOT NULL,
    audience character varying NOT NULL,
    base_main_oid character varying NOT NULL,
    head_oid character varying NOT NULL,
    git_snapshot jsonb,
    title text NOT NULL,
    description_markdown text NOT NULL,
    activity_version bigint NOT NULL,
    submitted_at_unix bigint,
    closed_at_unix bigint,
    closed_by_user_id character varying,
    merged_at_unix bigint,
    merged_by_user_id character varying,
    merged_head_oid character varying,
    merged_main_oid character varying,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_identity_values CHECK ((((name)::text ~ '^[a-z0-9][a-z0-9-]{0,47}$'::text) AND ((name)::text <> ALL ((ARRAY['main'::character varying, 'head'::character varying, 'scope'::character varying])::text[])) AND ((audience)::text = ANY ((ARRAY['Public'::character varying, 'Private'::character varying])::text[])) AND ((author_role)::text = ANY ((ARRAY['Public'::character varying, 'Member'::character varying, 'Owner'::character varying])::text[])))),
    CONSTRAINT scope_request_merge_coherence CHECK ((((merged_at_unix IS NULL) AND (merged_by_user_id IS NULL) AND (merged_head_oid IS NULL) AND (merged_main_oid IS NULL)) OR ((submitted_at_unix IS NOT NULL) AND (merged_at_unix IS NOT NULL) AND (merged_by_user_id IS NOT NULL) AND (merged_head_oid IS NOT NULL) AND (length((merged_head_oid)::text) > 0) AND (merged_main_oid IS NOT NULL) AND (length((merged_main_oid)::text) > 0)))),
    CONSTRAINT scope_request_nonnegative_values CHECK (((activity_version >= 0) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix) AND ((submitted_at_unix IS NULL) OR ((submitted_at_unix >= created_at_unix) AND (submitted_at_unix <= updated_at_unix))) AND ((closed_at_unix IS NULL) OR ((closed_at_unix >= created_at_unix) AND (closed_at_unix <= updated_at_unix))) AND ((merged_at_unix IS NULL) OR ((merged_at_unix >= created_at_unix) AND (merged_at_unix <= updated_at_unix))))),
    CONSTRAINT scope_request_submission_coherence CHECK ((((closed_at_unix IS NULL) OR (merged_at_unix IS NULL)) AND ((closed_at_unix IS NULL) = (closed_by_user_id IS NULL)) AND (((submitted_at_unix IS NULL) AND (closed_at_unix IS NULL) AND (merged_at_unix IS NULL)) OR ((submitted_at_unix IS NOT NULL) AND ((closed_at_unix IS NULL) OR (closed_at_unix >= submitted_at_unix)) AND ((merged_at_unix IS NULL) OR (merged_at_unix >= submitted_at_unix))))))
);

CREATE TABLE scope_run_attempt_cache_setups (
    attempt_id text NOT NULL,
    authorization_ms bigint NOT NULL,
    wall_ms bigint NOT NULL,
    CONSTRAINT scope_run_attempt_cache_setups_timings CHECK ((((authorization_ms >= 0) AND (authorization_ms <= 86400000)) AND ((wall_ms >= 0) AND (wall_ms <= 86400000)) AND (authorization_ms <= wall_ms)))
);

CREATE TABLE scope_run_attempt_caches (
    attempt_id text NOT NULL,
    identity_digest character varying(64) NOT NULL,
    workflow_path text NOT NULL,
    job_key character varying(64) NOT NULL,
    cache_name character varying(64) NOT NULL,
    preparation text NOT NULL,
    cold_reason text,
    prepare_ms bigint NOT NULL,
    final_state text NOT NULL,
    finalize_ms bigint,
    key_ms bigint NOT NULL,
    metadata_ms bigint NOT NULL,
    size_bytes bigint NOT NULL,
    download_verify_ms bigint NOT NULL,
    sync_ms bigint NOT NULL,
    extraction_ms bigint NOT NULL,
    CONSTRAINT scope_run_attempt_caches_finalization CHECK ((((final_state = 'pending'::text) AND (finalize_ms IS NULL)) OR ((final_state = ANY (ARRAY['ready'::text, 'evicted'::text])) AND ((finalize_ms >= 0) AND (finalize_ms <= 86400000))))),
    CONSTRAINT scope_run_attempt_caches_identity_digest CHECK (((identity_digest)::text ~ '^[0-9a-f]{64}$'::text)),
    CONSTRAINT scope_run_attempt_caches_preparation CHECK ((((preparation = ANY (ARRAY['exact'::text, 'compatible'::text])) AND (cold_reason IS NULL)) OR ((preparation = 'cold'::text) AND (cold_reason = ANY (ARRAY['metadata-missing'::text, 'metadata-invalid'::text, 'metadata-not-ready'::text, 'volume-missing'::text, 'volume-invalid'::text, 'backing-directory-missing'::text]))))),
    CONSTRAINT scope_run_attempt_caches_preparation_timings CHECK ((((key_ms >= 0) AND (key_ms <= 86400000)) AND ((metadata_ms >= 0) AND (metadata_ms <= 86400000)) AND ((size_bytes >= 0) AND (size_bytes <= 1073741824)) AND ((download_verify_ms >= 0) AND (download_verify_ms <= 86400000)) AND ((sync_ms >= 0) AND (sync_ms <= 86400000)) AND ((extraction_ms >= 0) AND (extraction_ms <= 86400000)) AND (prepare_ms = ((((key_ms + metadata_ms) + download_verify_ms) + sync_ms) + extraction_ms)))),
    CONSTRAINT scope_run_attempt_caches_prepare_duration CHECK (((prepare_ms >= 0) AND (prepare_ms <= 86400000)))
);

CREATE TABLE scope_run_attempt_steps (
    attempt_id character varying NOT NULL,
    step_index integer NOT NULL,
    state character varying NOT NULL,
    started_at_unix bigint,
    completed_at_unix bigint,
    exit_code integer,
    CONSTRAINT scope_run_attempt_steps_values CHECK (((step_index >= 0) AND ((state)::text = ANY ((ARRAY['pending'::character varying, 'running'::character varying, 'succeeded'::character varying, 'failed'::character varying, 'canceled'::character varying, 'lost'::character varying, 'skipped'::character varying])::text[])) AND (((state)::text = ANY ((ARRAY['succeeded'::character varying, 'failed'::character varying, 'canceled'::character varying, 'lost'::character varying, 'skipped'::character varying])::text[])) = (completed_at_unix IS NOT NULL)) AND ((started_at_unix IS NULL) OR (completed_at_unix IS NULL) OR (completed_at_unix >= started_at_unix)) AND (((state)::text <> 'pending'::text) OR ((started_at_unix IS NULL) AND (completed_at_unix IS NULL) AND (exit_code IS NULL))) AND (((state)::text <> 'running'::text) OR ((started_at_unix IS NOT NULL) AND (completed_at_unix IS NULL) AND (exit_code IS NULL))) AND (((state)::text <> 'succeeded'::text) OR ((started_at_unix IS NOT NULL) AND (exit_code = 0))) AND (((state)::text <> 'failed'::text) OR ((started_at_unix IS NOT NULL) AND (exit_code IS NOT NULL) AND (exit_code <> 0))) AND (((state)::text = ANY ((ARRAY['failed'::character varying, 'succeeded'::character varying])::text[])) OR (exit_code IS NULL)) AND (((state)::text <> 'skipped'::text) OR (started_at_unix IS NULL)) AND (((state)::text <> ALL ((ARRAY['canceled'::character varying, 'lost'::character varying])::text[])) OR (started_at_unix IS NOT NULL))))
);

CREATE TABLE scope_run_attempts (
    id character varying NOT NULL,
    run_id character varying NOT NULL,
    number integer NOT NULL,
    token_hash character varying NOT NULL,
    token_expires_at_unix bigint NOT NULL,
    state character varying NOT NULL,
    lease_expires_at_unix bigint NOT NULL,
    last_heartbeat_at_unix bigint NOT NULL,
    created_at_unix bigint NOT NULL,
    started_at_unix bigint,
    completed_at_unix bigint,
    terminal_reason jsonb,
    log_bytes bigint NOT NULL,
    job_key character varying NOT NULL,
    first_truncated_step_index integer,
    external_run_id text,
    runner_stop_claimed_at_unix bigint,
    runtime_version text NOT NULL,
    runner_stop_completed_at_unix bigint,
    CONSTRAINT scope_run_attempts_truncated_step_nonnegative CHECK (((first_truncated_step_index IS NULL) OR (first_truncated_step_index >= 0))),
    CONSTRAINT scope_run_attempts_values CHECK (((number > 0) AND ((external_run_id IS NULL) OR (char_length(external_run_id) > 0)) AND ((runner_stop_claimed_at_unix IS NULL) OR (runner_stop_claimed_at_unix >= created_at_unix)) AND ((runner_stop_completed_at_unix IS NULL) OR ((runner_stop_claimed_at_unix IS NOT NULL) AND (runner_stop_completed_at_unix >= runner_stop_claimed_at_unix))) AND ((char_length(runtime_version) >= 1) AND (char_length(runtime_version) <= 128)) AND (char_length((token_hash)::text) = 64) AND ((token_hash)::text ~ '^[0-9A-Fa-f]+$'::text) AND ((state)::text = ANY ((ARRAY['dispatching'::character varying, 'running'::character varying, 'succeeded'::character varying, 'failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) AND (token_expires_at_unix = lease_expires_at_unix) AND (created_at_unix >= 0) AND (last_heartbeat_at_unix >= created_at_unix) AND (last_heartbeat_at_unix < lease_expires_at_unix) AND ((started_at_unix IS NULL) OR ((started_at_unix >= created_at_unix) AND (started_at_unix < lease_expires_at_unix))) AND ((completed_at_unix IS NULL) OR (completed_at_unix >= last_heartbeat_at_unix)) AND ((started_at_unix IS NULL) OR (completed_at_unix IS NULL) OR (completed_at_unix >= started_at_unix)) AND (log_bytes >= 0) AND (log_bytes <= 10485760) AND (((state)::text = ANY ((ARRAY['succeeded'::character varying, 'failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) = (completed_at_unix IS NOT NULL)) AND (((state)::text <> 'succeeded'::text) OR ((started_at_unix IS NOT NULL) AND (terminal_reason IS NULL))) AND (((state)::text <> ALL ((ARRAY['failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) OR (terminal_reason IS NOT NULL)) AND (((state)::text = ANY ((ARRAY['failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) OR (terminal_reason IS NULL))))
);

CREATE TABLE scope_runs (
    id character varying NOT NULL,
    idempotency_key character varying NOT NULL,
    repo_id character varying NOT NULL,
    workflow_path text NOT NULL,
    workflow_revision_digest character varying NOT NULL,
    trigger character varying NOT NULL,
    requested_by_user_id character varying,
    source jsonb NOT NULL,
    state character varying NOT NULL,
    cancellation_requested boolean NOT NULL,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    completed_at_unix bigint,
    creation_sequence bigint NOT NULL,
    CONSTRAINT scope_runs_values CHECK (((char_length((workflow_revision_digest)::text) = 64) AND ((workflow_revision_digest)::text ~ '^[0-9A-Fa-f]+$'::text) AND ((((source ->> 'kind'::text) = 'ephemeral-git-bundle'::text) AND (char_length((source #>> '{object,sha256}'::text[])) = 64) AND ((source #>> '{object,sha256}'::text[]) ~ '^[0-9A-Fa-f]+$'::text) AND (char_length((source #>> '{object,git_oid}'::text[])) = 40) AND ((source #>> '{object,git_oid}'::text[]) ~ '^[0-9A-Fa-f]+$'::text)) OR (((source ->> 'kind'::text) = 'accepted-git-head'::text) AND (length(btrim((source ->> 'repository_id'::text))) > 0) AND ((source ->> 'audience'::text) = ANY (ARRAY['Private'::text, 'Public'::text])) AND (((source #>> '{head,push_sequence}'::text[]))::numeric > (0)::numeric) AND (((source #>> '{head,change_version}'::text[]))::numeric > (0)::numeric) AND (char_length((source #>> '{head,head_oid}'::text[])) = 40) AND ((source #>> '{head,head_oid}'::text[]) ~ '^[0-9A-Fa-f]+$'::text) AND (char_length((source #>> '{head,manifest,sha256}'::text[])) = 64) AND ((source #>> '{head,manifest,sha256}'::text[]) ~ '^[0-9A-Fa-f]+$'::text) AND (char_length((source #>> '{head,manifest,git_oid}'::text[])) = 40) AND ((source #>> '{head,manifest,git_oid}'::text[]) ~ '^[0-9A-Fa-f]+$'::text) AND ((source #>> '{head,manifest,git_oid}'::text[]) = (source #>> '{head,head_oid}'::text[])) AND (jsonb_typeof((source -> 'pack_spans'::text)) = 'array'::text) AND (jsonb_array_length((source -> 'pack_spans'::text)) > 0) AND (((((source -> 'pack_spans'::text) -> (jsonb_array_length((source -> 'pack_spans'::text)) - 1)) ->> 'last_sequence'::text))::numeric = ((source #>> '{head,push_sequence}'::text[]))::numeric) AND ((((source -> 'pack_spans'::text) -> (jsonb_array_length((source -> 'pack_spans'::text)) - 1)) ->> 'head_oid'::text) = (source #>> '{head,head_oid}'::text[])))) AND ((trigger)::text = ANY ((ARRAY['manual'::character varying, 'push-main'::character varying])::text[])) AND ((state)::text = ANY ((ARRAY['queued'::character varying, 'dispatching'::character varying, 'running'::character varying, 'succeeded'::character varying, 'failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix) AND (((state)::text = ANY ((ARRAY['succeeded'::character varying, 'failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) = (completed_at_unix IS NOT NULL)) AND ((completed_at_unix IS NULL) OR (completed_at_unix = updated_at_unix)) AND (((state)::text <> 'canceled'::text) OR cancellation_requested)))
);

CREATE SEQUENCE scope_run_creation_sequence
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE scope_run_creation_sequence OWNED BY scope_runs.creation_sequence;

CREATE TABLE scope_run_jobs (
    run_id character varying NOT NULL,
    job_key character varying NOT NULL,
    pinned_container_image text NOT NULL,
    state character varying NOT NULL,
    last_attempt_number integer NOT NULL,
    current_attempt_id character varying,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    completed_at_unix bigint,
    CONSTRAINT scope_run_jobs_values CHECK ((((char_length((job_key)::text) >= 1) AND (char_length((job_key)::text) <= 64)) AND ((job_key)::text ~ '^[a-z0-9]+(-[a-z0-9]+)*$'::text) AND (pinned_container_image ~ '^[^@[:space:]]+@sha256:[0-9A-Fa-f]{64}$'::text) AND ((state)::text = ANY ((ARRAY['blocked'::character varying, 'queued'::character varying, 'dispatching'::character varying, 'running'::character varying, 'succeeded'::character varying, 'failed'::character varying, 'skipped'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) AND (last_attempt_number >= 0) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix) AND (((state)::text = ANY ((ARRAY['dispatching'::character varying, 'running'::character varying])::text[])) = (current_attempt_id IS NOT NULL)) AND (((state)::text = ANY ((ARRAY['succeeded'::character varying, 'failed'::character varying, 'skipped'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) = (completed_at_unix IS NOT NULL)) AND ((completed_at_unix IS NULL) OR (completed_at_unix = updated_at_unix))))
);

CREATE TABLE scope_run_logs (
    "position" bigint NOT NULL,
    run_id character varying NOT NULL,
    attempt_id character varying NOT NULL,
    step_index integer NOT NULL,
    sequence bigint NOT NULL,
    text text NOT NULL,
    created_at_unix bigint NOT NULL,
    CONSTRAINT scope_run_logs_values CHECK ((("position" > 0) AND (sequence > 0) AND ((octet_length(text) >= 1) AND (octet_length(text) <= 65536)) AND (created_at_unix >= 0)))
);

ALTER TABLE scope_run_logs ALTER COLUMN "position" ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME scope_run_logs_position_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE scope_users (
    id character varying NOT NULL,
    handle character varying NOT NULL,
    email character varying NOT NULL,
    email_verified boolean NOT NULL
);

CREATE TABLE scope_visibility_change_sets (
    repo_id character varying NOT NULL,
    id character varying NOT NULL,
    ordinal bigint NOT NULL,
    anchor_commit_id character varying,
    source_update_id character varying,
    author_id character varying NOT NULL,
    occurred_at_unix bigint,
    CONSTRAINT scope_visibility_change_set_values CHECK (((ordinal >= 0) AND (char_length((id)::text) > 0) AND (char_length((author_id)::text) > 0)))
);

CREATE TABLE scope_visibility_changes (
    repo_id character varying NOT NULL,
    change_set_id character varying NOT NULL,
    ordinal bigint NOT NULL,
    path character varying NOT NULL,
    old_visibility character varying NOT NULL,
    new_visibility character varying NOT NULL,
    current_content jsonb,
    CONSTRAINT scope_visibility_change_values CHECK (((ordinal >= 0) AND (char_length((path)::text) > 0) AND ((old_visibility)::text = ANY ((ARRAY['Public'::character varying, 'Private'::character varying])::text[])) AND ((new_visibility)::text = ANY ((ARRAY['Public'::character varying, 'Private'::character varying])::text[])) AND ((old_visibility)::text <> (new_visibility)::text)))
);

CREATE TABLE scope_workflow_revisions (
    digest character varying NOT NULL,
    definition jsonb NOT NULL,
    created_at_unix bigint NOT NULL,
    CONSTRAINT scope_workflow_revisions_jobs_shape CHECK (((jsonb_typeof(definition) = 'object'::text) AND (definition ? 'jobs'::text) AND (jsonb_typeof((definition -> 'jobs'::text)) = 'array'::text) AND ((jsonb_array_length((definition -> 'jobs'::text)) >= 1) AND (jsonb_array_length((definition -> 'jobs'::text)) <= 64)) AND (NOT (definition ?| ARRAY['runner'::text, 'container'::text, 'timeout_seconds'::text, 'caches'::text, 'steps'::text])))),
    CONSTRAINT scope_workflow_revisions_values CHECK (((char_length((digest)::text) = 64) AND ((digest)::text ~ '^[0-9A-Fa-f]+$'::text) AND (created_at_unix >= 0)))
);

ALTER TABLE ONLY scope_runs ALTER COLUMN creation_sequence SET DEFAULT nextval('scope_run_creation_sequence'::regclass);

ALTER TABLE ONLY scope_auth_identities
    ADD CONSTRAINT pk_scope_auth_identities PRIMARY KEY (provider, subject);

ALTER TABLE ONLY scope_projection_files
    ADD CONSTRAINT pk_scope_projection_files PRIMARY KEY (repo_id, source, audience, path_key);

ALTER TABLE ONLY scope_projection_read_models
    ADD CONSTRAINT pk_scope_projection_read_models PRIMARY KEY (repo_id, source, audience);

ALTER TABLE ONLY scope_repository_members
    ADD CONSTRAINT pk_scope_repository_members PRIMARY KEY (repo_id, user_id);

ALTER TABLE ONLY scope_cache_deletion_queue
    ADD CONSTRAINT scope_cache_deletion_queue_pkey PRIMARY KEY (repository_id, checksum_sha256);

ALTER TABLE ONLY scope_cache_objects
    ADD CONSTRAINT scope_cache_objects_object_key_key UNIQUE (object_key);

ALTER TABLE ONLY scope_cache_objects
    ADD CONSTRAINT scope_cache_objects_pkey PRIMARY KEY (repository_id, checksum_sha256);

ALTER TABLE ONLY scope_cache_orphan_uploads
    ADD CONSTRAINT scope_cache_orphan_uploads_pkey PRIMARY KEY (object_key);

ALTER TABLE ONLY scope_cache_references
    ADD CONSTRAINT scope_cache_references_pkey PRIMARY KEY (repository_id, identity_digest);

ALTER TABLE ONLY scope_cache_uploads
    ADD CONSTRAINT scope_cache_uploads_object_key_key UNIQUE (object_key);

ALTER TABLE ONLY scope_cache_uploads
    ADD CONSTRAINT scope_cache_uploads_pkey PRIMARY KEY (upload_id);

ALTER TABLE ONLY scope_cli_browser_logins
    ADD CONSTRAINT scope_cli_browser_logins_pkey PRIMARY KEY (request_id);

ALTER TABLE ONLY scope_cli_device_logins
    ADD CONSTRAINT scope_cli_device_logins_pkey PRIMARY KEY (device_code_hash);

ALTER TABLE ONLY scope_cli_device_logins
    ADD CONSTRAINT scope_cli_device_logins_user_code_hash_key UNIQUE (user_code_hash);

ALTER TABLE ONLY scope_cli_exchange_grants
    ADD CONSTRAINT scope_cli_exchange_grants_pkey PRIMARY KEY (grant_hash);

ALTER TABLE ONLY scope_cli_sessions
    ADD CONSTRAINT scope_cli_sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_cli_sessions
    ADD CONSTRAINT scope_cli_sessions_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY scope_file_changes
    ADD CONSTRAINT scope_file_changes_pkey PRIMARY KEY (repo_id, commit_id, ordinal);

ALTER TABLE ONLY scope_git_compaction_jobs
    ADD CONSTRAINT scope_git_compaction_jobs_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_git_heads
    ADD CONSTRAINT scope_git_heads_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_git_segments
    ADD CONSTRAINT scope_git_pack_spans_pkey PRIMARY KEY (repo_id, first_sequence);

ALTER TABLE ONLY scope_git_segment_references
    ADD CONSTRAINT scope_git_segment_references_pkey PRIMARY KEY (segment_id, ref_kind, ref_id);

ALTER TABLE ONLY scope_git_segment_uploads
    ADD CONSTRAINT scope_git_segment_uploads_object_key_key UNIQUE (object_key);

ALTER TABLE ONLY scope_git_segment_uploads
    ADD CONSTRAINT scope_git_segment_uploads_pkey PRIMARY KEY (segment_id);

ALTER TABLE ONLY scope_live_files
    ADD CONSTRAINT scope_live_files_pkey PRIMARY KEY (repo_id, path);

ALTER TABLE ONLY scope_logical_commits
    ADD CONSTRAINT scope_logical_commits_pkey PRIMARY KEY (repo_id, id);

ALTER TABLE ONLY scope_logical_commits
    ADD CONSTRAINT scope_logical_commits_repo_id_ordinal_key UNIQUE (repo_id, ordinal);

ALTER TABLE ONLY scope_metadata_locks
    ADD CONSTRAINT scope_metadata_locks_pkey PRIMARY KEY (key);

ALTER TABLE ONLY scope_object_references
    ADD CONSTRAINT scope_object_references_pkey PRIMARY KEY (object_key, ref_kind, ref_id);

ALTER TABLE ONLY scope_orphan_object_jobs
    ADD CONSTRAINT scope_orphan_object_jobs_pkey PRIMARY KEY (object_key);

ALTER TABLE ONLY scope_outbox_jobs
    ADD CONSTRAINT scope_outbox_jobs_idempotency_key_key UNIQUE (idempotency_key);

ALTER TABLE ONLY scope_outbox_jobs
    ADD CONSTRAINT scope_outbox_jobs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_push_trigger_evaluations
    ADD CONSTRAINT scope_push_trigger_evaluations_pkey PRIMARY KEY (repo_id, change_version);

ALTER TABLE ONLY scope_repo_storage_cleanup_jobs
    ADD CONSTRAINT scope_repo_storage_cleanup_jobs_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_repositories
    ADD CONSTRAINT scope_repositories_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_repository_first_push_tokens
    ADD CONSTRAINT scope_repository_first_push_tokens_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_repository_git_push_tokens
    ADD CONSTRAINT scope_repository_git_push_tokens_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_repository_history_entries
    ADD CONSTRAINT scope_repository_history_entries_pkey PRIMARY KEY (repo_id, audience, "position");

ALTER TABLE ONLY scope_repository_history_entries
    ADD CONSTRAINT scope_repository_history_entries_repo_id_audience_source_id_key UNIQUE (repo_id, audience, source_id);

ALTER TABLE ONLY scope_repository_history_views
    ADD CONSTRAINT scope_repository_history_views_pkey PRIMARY KEY (repo_id, audience);

ALTER TABLE ONLY scope_repository_invites
    ADD CONSTRAINT scope_repository_invites_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_repository_landing_files
    ADD CONSTRAINT scope_repository_landing_files_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_repository_workflow_catalogs
    ADD CONSTRAINT scope_repository_workflow_catalogs_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_repository_workflow_files
    ADD CONSTRAINT scope_repository_workflow_files_pkey PRIMARY KEY (repo_id, path);

ALTER TABLE ONLY scope_request_discussion_read_states
    ADD CONSTRAINT scope_request_discussion_read_states_pkey PRIMARY KEY (discussion_id, user_id);

ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT scope_request_discussion_replies_client_key UNIQUE (discussion_id, author_user_id, client_reply_id);

ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT scope_request_discussion_replies_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT scope_request_discussion_replies_position_key UNIQUE (discussion_id, "position");

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT scope_request_discussions_client_key UNIQUE (request_id, author_user_id, client_discussion_id);

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT scope_request_discussions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT scope_request_discussions_position_key UNIQUE (request_id, opened_position);

ALTER TABLE ONLY scope_request_events
    ADD CONSTRAINT scope_request_events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT scope_request_invitees_pkey PRIMARY KEY (request_id, user_id);

ALTER TABLE ONLY scope_request_media_abandoned_objects
    ADD CONSTRAINT scope_request_media_abandoned_objects_pkey PRIMARY KEY (object_key);

ALTER TABLE ONLY scope_request_media_attachments
    ADD CONSTRAINT scope_request_media_attachmen_request_id_uploader_user_id_o_key UNIQUE (request_id, uploader_user_id, operation_id);

ALTER TABLE ONLY scope_request_media_attachments
    ADD CONSTRAINT scope_request_media_attachments_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_request_media_attachments
    ADD CONSTRAINT scope_request_media_attachments_upload_id_key UNIQUE (upload_id);

ALTER TABLE ONLY scope_request_media_bindings
    ADD CONSTRAINT scope_request_media_bindings_pkey PRIMARY KEY (attachment_id, target_key);

ALTER TABLE ONLY scope_request_media_cleanup_jobs
    ADD CONSTRAINT scope_request_media_cleanup_jobs_pkey PRIMARY KEY (attachment_id);

ALTER TABLE ONLY scope_request_media_derivatives
    ADD CONSTRAINT scope_request_media_derivatives_attachment_id_kind_key UNIQUE (attachment_id, kind);

ALTER TABLE ONLY scope_request_media_derivatives
    ADD CONSTRAINT scope_request_media_derivatives_manifest_id_key UNIQUE (manifest_id);

ALTER TABLE ONLY scope_request_media_derivatives
    ADD CONSTRAINT scope_request_media_derivatives_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_request_media_manifest_chunks
    ADD CONSTRAINT scope_request_media_manifest_chunks_manifest_id_object_key_key UNIQUE (manifest_id, object_key);

ALTER TABLE ONLY scope_request_media_manifest_chunks
    ADD CONSTRAINT scope_request_media_manifest_chunks_pkey PRIMARY KEY (manifest_id, chunk_index);

ALTER TABLE ONLY scope_request_media_manifests
    ADD CONSTRAINT scope_request_media_manifests_attachment_id_derivative_id_key UNIQUE (attachment_id, derivative_id);

ALTER TABLE ONLY scope_request_media_manifests
    ADD CONSTRAINT scope_request_media_manifests_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_request_media_orphan_cleanup_leases
    ADD CONSTRAINT scope_request_media_orphan_cleanup_leases_pkey PRIMARY KEY (attachment_id);

ALTER TABLE ONLY scope_request_media_processing_jobs
    ADD CONSTRAINT scope_request_media_processing_jobs_pkey PRIMARY KEY (attachment_id);

ALTER TABLE ONLY scope_request_media_processing_objects
    ADD CONSTRAINT scope_request_media_processing_objects_pkey PRIMARY KEY (object_key);

ALTER TABLE ONLY scope_request_media_retry_operations
    ADD CONSTRAINT scope_request_media_retry_operations_pkey PRIMARY KEY (attachment_id, operation_id);

ALTER TABLE ONLY scope_request_media_upload_parts
    ADD CONSTRAINT scope_request_media_upload_parts_object_key_key UNIQUE (object_key);

ALTER TABLE ONLY scope_request_media_upload_parts
    ADD CONSTRAINT scope_request_media_upload_parts_pkey PRIMARY KEY (attachment_id, part_number);

ALTER TABLE ONLY scope_request_ratings
    ADD CONSTRAINT scope_request_rating_one_per_rater UNIQUE (request_id, rater_user_id);

ALTER TABLE ONLY scope_request_ratings
    ADD CONSTRAINT scope_request_rating_one_per_subject UNIQUE (request_id, subject_user_id);

ALTER TABLE ONLY scope_request_ratings
    ADD CONSTRAINT scope_request_ratings_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_request_revisions
    ADD CONSTRAINT scope_request_revisions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_request_revisions
    ADD CONSTRAINT scope_request_revisions_position_key UNIQUE (request_id, "position");

ALTER TABLE ONLY scope_request_revisions
    ADD CONSTRAINT scope_request_revisions_request_id_key UNIQUE (request_id, id);

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT scope_requests_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT scope_requests_repo_name_key UNIQUE (repo_id, name);

ALTER TABLE ONLY scope_run_attempt_cache_setups
    ADD CONSTRAINT scope_run_attempt_cache_setups_pkey PRIMARY KEY (attempt_id);

ALTER TABLE ONLY scope_run_attempt_caches
    ADD CONSTRAINT scope_run_attempt_caches_pkey PRIMARY KEY (attempt_id, identity_digest);

ALTER TABLE ONLY scope_run_attempt_steps
    ADD CONSTRAINT scope_run_attempt_steps_pkey PRIMARY KEY (attempt_id, step_index);

ALTER TABLE ONLY scope_run_attempts
    ADD CONSTRAINT scope_run_attempts_identity_key UNIQUE (id, run_id, job_key);

ALTER TABLE ONLY scope_run_attempts
    ADD CONSTRAINT scope_run_attempts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_run_attempts
    ADD CONSTRAINT scope_run_attempts_run_id_job_key_number_key UNIQUE (run_id, job_key, number);

ALTER TABLE ONLY scope_run_attempts
    ADD CONSTRAINT scope_run_attempts_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY scope_run_jobs
    ADD CONSTRAINT scope_run_jobs_pkey PRIMARY KEY (run_id, job_key);

ALTER TABLE ONLY scope_run_logs
    ADD CONSTRAINT scope_run_logs_attempt_id_sequence_key UNIQUE (attempt_id, sequence);

ALTER TABLE ONLY scope_run_logs
    ADD CONSTRAINT scope_run_logs_pkey PRIMARY KEY ("position");

ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT scope_runs_creation_sequence_unique UNIQUE (creation_sequence);

ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT scope_runs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT scope_runs_repo_id_idempotency_key_key UNIQUE (repo_id, idempotency_key);

ALTER TABLE ONLY scope_users
    ADD CONSTRAINT scope_users_handle_key UNIQUE (handle);

ALTER TABLE ONLY scope_users
    ADD CONSTRAINT scope_users_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_visibility_change_sets
    ADD CONSTRAINT scope_visibility_change_sets_pkey PRIMARY KEY (repo_id, id);

ALTER TABLE ONLY scope_visibility_change_sets
    ADD CONSTRAINT scope_visibility_change_sets_repo_id_ordinal_key UNIQUE (repo_id, ordinal);

ALTER TABLE ONLY scope_visibility_changes
    ADD CONSTRAINT scope_visibility_changes_pkey PRIMARY KEY (repo_id, change_set_id, ordinal);

ALTER TABLE ONLY scope_visibility_changes
    ADD CONSTRAINT scope_visibility_changes_repo_id_change_set_id_path_key UNIQUE (repo_id, change_set_id, path);

ALTER TABLE ONLY scope_workflow_revisions
    ADD CONSTRAINT scope_workflow_revisions_pkey PRIMARY KEY (digest);

ALTER TABLE ONLY scope_git_segments
    ADD CONSTRAINT uq_scope_git_segments_segment UNIQUE (segment_id);

ALTER TABLE ONLY scope_repo_storage_cleanup_jobs
    ADD CONSTRAINT uq_scope_repo_cleanup_incarnation UNIQUE (incarnation_id);

ALTER TABLE ONLY scope_repositories
    ADD CONSTRAINT uq_scope_repositories_incarnation UNIQUE (incarnation_id);

CREATE INDEX idx_scope_auth_identities_user ON scope_auth_identities USING btree (user_id);

CREATE INDEX idx_scope_cache_deletion_queue_due ON scope_cache_deletion_queue USING btree (not_before_unix, repository_id, checksum_sha256);

CREATE INDEX idx_scope_cache_objects_access ON scope_cache_objects USING btree (repository_id, last_accessed_at_unix, checksum_sha256);

CREATE INDEX idx_scope_cache_orphan_uploads_due ON scope_cache_orphan_uploads USING btree (not_before_unix, object_key);

CREATE INDEX idx_scope_cache_references_access ON scope_cache_references USING btree (repository_id, last_accessed_at_unix, identity_digest);

CREATE INDEX idx_scope_cache_references_compatibility ON scope_cache_references USING btree (repository_id, compatibility_group_digest, created_at_unix DESC, identity_digest);

CREATE INDEX idx_scope_cache_references_expiry ON scope_cache_references USING btree (expires_at_unix, repository_id, identity_digest);

CREATE INDEX idx_scope_cache_references_object ON scope_cache_references USING btree (repository_id, checksum_sha256);

CREATE UNIQUE INDEX idx_scope_cache_uploads_active_identity ON scope_cache_uploads USING btree (repository_id, identity_digest) WHERE (state = ANY (ARRAY['active'::text, 'deleting'::text]));

CREATE INDEX idx_scope_cache_uploads_expiry ON scope_cache_uploads USING btree (expires_at_unix, upload_id);

CREATE INDEX idx_scope_cli_exchange_grants_user ON scope_cli_exchange_grants USING btree (user_id);

CREATE INDEX idx_scope_cli_sessions_user ON scope_cli_sessions USING btree (user_id);

CREATE INDEX idx_scope_git_segment_references_owner ON scope_git_segment_references USING btree (ref_kind, ref_id, segment_id);

CREATE INDEX idx_scope_git_segment_uploads_recovery ON scope_git_segment_uploads USING btree (state, updated_at_unix, segment_id) WHERE (state = ANY (ARRAY['uploading'::text, 'ready'::text, 'deleting'::text]));

CREATE INDEX idx_scope_orphan_object_jobs_pending ON scope_orphan_object_jobs USING btree (completed_at_unix, next_run_at_unix);

CREATE INDEX idx_scope_outbox_jobs_ready ON scope_outbox_jobs USING btree (state, next_run_at_unix, created_at_unix);

CREATE INDEX idx_scope_outbox_jobs_repo ON scope_outbox_jobs USING btree (repo_id, repo_version);

CREATE INDEX idx_scope_projection_files_lookup ON scope_projection_files USING btree (repo_id, repo_version, source, audience);

CREATE INDEX idx_scope_repo_storage_cleanup_jobs_pending ON scope_repo_storage_cleanup_jobs USING btree (completed_at_unix, next_run_at_unix);

CREATE UNIQUE INDEX idx_scope_repositories_owner_name ON scope_repositories USING btree (owner_handle, name);

CREATE INDEX idx_scope_repository_invites_repo_email ON scope_repository_invites USING btree (repo_id, invited_email_normalized);

CREATE INDEX idx_scope_repository_invites_token_hash ON scope_repository_invites USING btree (token_hash);

CREATE INDEX idx_scope_repository_members_user ON scope_repository_members USING btree (user_id);

CREATE INDEX idx_scope_request_discussion_replies_chronological ON scope_request_discussion_replies USING btree (discussion_id, "position" DESC);

CREATE INDEX idx_scope_request_discussions_newest ON scope_request_discussions USING btree (request_id, opened_position DESC, id);

CREATE UNIQUE INDEX idx_scope_request_events_request_position ON scope_request_events USING btree (request_id, "position");

CREATE INDEX idx_scope_request_invitees_user ON scope_request_invitees USING btree (user_id, request_id);

CREATE INDEX idx_scope_request_ratings_subject ON scope_request_ratings USING btree (subject_user_id, created_at_unix, id);

CREATE INDEX idx_scope_request_revisions_request_position ON scope_request_revisions USING btree (request_id, "position" DESC, id);

CREATE INDEX idx_scope_requests_author ON scope_requests USING btree (author_user_id);

CREATE INDEX idx_scope_requests_closed_queue ON scope_requests USING btree (repo_id, COALESCE(closed_at_unix, merged_at_unix) DESC, id) WHERE ((closed_at_unix IS NOT NULL) OR (merged_at_unix IS NOT NULL));

CREATE INDEX idx_scope_requests_draft_queue ON scope_requests USING btree (repo_id, updated_at_unix DESC, id) WHERE ((submitted_at_unix IS NULL) AND (closed_at_unix IS NULL) AND (merged_at_unix IS NULL));

CREATE INDEX idx_scope_requests_open_queue ON scope_requests USING btree (repo_id, submitted_at_unix, id) WHERE ((submitted_at_unix IS NOT NULL) AND (closed_at_unix IS NULL) AND (merged_at_unix IS NULL));

CREATE INDEX idx_scope_requests_public_closed_queue ON scope_requests USING btree (repo_id, COALESCE(closed_at_unix, merged_at_unix) DESC, id) WHERE (((audience)::text = 'Public'::text) AND ((closed_at_unix IS NOT NULL) OR (merged_at_unix IS NOT NULL)));

CREATE INDEX idx_scope_requests_public_open_queue ON scope_requests USING btree (repo_id, submitted_at_unix, id) WHERE (((audience)::text = 'Public'::text) AND (submitted_at_unix IS NOT NULL) AND (closed_at_unix IS NULL) AND (merged_at_unix IS NULL));

CREATE INDEX idx_scope_requests_public_search ON scope_requests USING gin (title public.gin_trgm_ops, description_markdown public.gin_trgm_ops) WHERE ((audience)::text = 'Public'::text);

CREATE INDEX idx_scope_requests_repo_audience_id ON scope_requests USING btree (repo_id, audience, id);

CREATE INDEX idx_scope_requests_repo_id ON scope_requests USING btree (repo_id, id);

CREATE UNIQUE INDEX idx_scope_run_attempts_active ON scope_run_attempts USING btree (run_id, job_key) WHERE ((state)::text = ANY ((ARRAY['leased'::character varying, 'running'::character varying])::text[]));

CREATE INDEX idx_scope_run_attempts_expiring ON scope_run_attempts USING btree (lease_expires_at_unix, id) WHERE ((state)::text = ANY ((ARRAY['leased'::character varying, 'running'::character varying])::text[]));

CREATE UNIQUE INDEX idx_scope_run_attempts_external_run ON scope_run_attempts USING btree (external_run_id) WHERE (external_run_id IS NOT NULL);

CREATE INDEX idx_scope_run_attempts_state ON scope_run_attempts USING btree (state, created_at_unix);

CREATE INDEX idx_scope_run_jobs_dispatch ON scope_run_jobs USING btree (created_at_unix, run_id, job_key) WHERE ((state)::text = 'queued'::text);

CREATE INDEX idx_scope_run_logs_run_position ON scope_run_logs USING btree (run_id, "position");

CREATE INDEX idx_scope_run_logs_step_position ON scope_run_logs USING btree (attempt_id, step_index, "position");

CREATE INDEX idx_scope_runs_history ON scope_runs USING btree (repo_id, creation_sequence DESC);

CREATE INDEX idx_scope_runs_queue ON scope_runs USING btree (created_at_unix, id) WHERE ((state)::text = 'queued'::text);

CREATE INDEX idx_scope_runs_workflow_history ON scope_runs USING btree (repo_id, workflow_path, creation_sequence DESC);

CREATE UNIQUE INDEX idx_scope_users_email ON scope_users USING btree (email);

CREATE INDEX scope_git_compaction_jobs_due ON scope_git_compaction_jobs USING btree (next_run_at_unix, lease_expires_at_unix, repo_id);

CREATE UNIQUE INDEX scope_git_pack_spans_last_sequence ON scope_git_segments USING btree (repo_id, last_sequence);

CREATE INDEX scope_object_references_owner ON scope_object_references USING btree (ref_kind, ref_id);

CREATE INDEX scope_request_media_abandoned_objects_cleanup_idx ON scope_request_media_abandoned_objects USING btree (attachment_id, deleted_at_unix);

CREATE INDEX scope_request_media_attachments_expiry_idx ON scope_request_media_attachments USING btree (upload_expires_at_unix, unbound_expires_at_unix);

CREATE INDEX scope_request_media_attachments_repository_usage_idx ON scope_request_media_attachments USING btree (repository_id, state);

CREATE INDEX scope_request_media_attachments_request_idx ON scope_request_media_attachments USING btree (request_id, id);

CREATE INDEX scope_request_media_bindings_target_idx ON scope_request_media_bindings USING btree (request_id, target_key, attachment_id);

CREATE INDEX scope_request_media_cleanup_claim_idx ON scope_request_media_cleanup_jobs USING btree (state, available_at_unix, lease_expires_at_unix);

CREATE INDEX scope_request_media_processing_claim_idx ON scope_request_media_processing_jobs USING btree (state, available_at_unix, lease_expires_at_unix);

CREATE INDEX scope_request_media_processing_objects_cleanup_idx ON scope_request_media_processing_objects USING btree (state, attachment_id, lease_generation);

CREATE TRIGGER scope_repository_workflow_catalog_rejection_guard BEFORE UPDATE OF configuration_error ON scope_repository_workflow_catalogs FOR EACH ROW EXECUTE FUNCTION scope_check_repository_workflow_catalog_rejection();

CREATE TRIGGER scope_repository_workflow_file_guard BEFORE INSERT OR UPDATE ON scope_repository_workflow_files FOR EACH ROW EXECUTE FUNCTION scope_check_repository_workflow_file();

CREATE TRIGGER scope_request_media_manifest_chunks_immutable BEFORE DELETE OR UPDATE ON scope_request_media_manifest_chunks FOR EACH ROW EXECUTE FUNCTION scope_reject_request_media_manifest_mutation();

CREATE TRIGGER scope_request_media_manifests_immutable BEFORE DELETE OR UPDATE ON scope_request_media_manifests FOR EACH ROW EXECUTE FUNCTION scope_reject_request_media_manifest_mutation();

ALTER TABLE ONLY scope_auth_identities
    ADD CONSTRAINT fk_scope_auth_identities_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_cache_deletion_queue
    ADD CONSTRAINT fk_scope_cache_deletion_queue_object FOREIGN KEY (repository_id, checksum_sha256) REFERENCES scope_cache_objects(repository_id, checksum_sha256) ON DELETE CASCADE;

ALTER TABLE ONLY scope_cache_references
    ADD CONSTRAINT fk_scope_cache_references_object FOREIGN KEY (repository_id, checksum_sha256) REFERENCES scope_cache_objects(repository_id, checksum_sha256) ON DELETE CASCADE;

ALTER TABLE ONLY scope_cli_browser_logins
    ADD CONSTRAINT fk_scope_cli_browser_logins_completed_user FOREIGN KEY (completed_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_cli_device_logins
    ADD CONSTRAINT fk_scope_cli_device_logins_completed_user FOREIGN KEY (completed_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_cli_exchange_grants
    ADD CONSTRAINT fk_scope_cli_exchange_grants_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_cli_sessions
    ADD CONSTRAINT fk_scope_cli_sessions_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_file_changes
    ADD CONSTRAINT fk_scope_file_changes_commit FOREIGN KEY (repo_id, commit_id) REFERENCES scope_logical_commits(repo_id, id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_git_heads
    ADD CONSTRAINT fk_scope_git_heads_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_git_segments
    ADD CONSTRAINT fk_scope_git_segments_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_git_segments
    ADD CONSTRAINT fk_scope_git_segments_upload FOREIGN KEY (segment_id) REFERENCES scope_git_segment_uploads(segment_id);

ALTER TABLE ONLY scope_live_files
    ADD CONSTRAINT fk_scope_live_files_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_logical_commits
    ADD CONSTRAINT fk_scope_logical_commits_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_outbox_jobs
    ADD CONSTRAINT fk_scope_outbox_jobs_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_projection_files
    ADD CONSTRAINT fk_scope_projection_files_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_projection_read_models
    ADD CONSTRAINT fk_scope_projection_read_models_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_push_trigger_evaluations
    ADD CONSTRAINT fk_scope_push_trigger_evaluations_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repositories
    ADD CONSTRAINT fk_scope_repositories_owner FOREIGN KEY (owner_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_first_push_tokens
    ADD CONSTRAINT fk_scope_repository_first_push_tokens_owner FOREIGN KEY (owner_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_first_push_tokens
    ADD CONSTRAINT fk_scope_repository_first_push_tokens_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_git_push_tokens
    ADD CONSTRAINT fk_scope_repository_git_push_tokens_owner FOREIGN KEY (owner_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_git_push_tokens
    ADD CONSTRAINT fk_scope_repository_git_push_tokens_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_invites
    ADD CONSTRAINT fk_scope_repository_invites_accepted_user FOREIGN KEY (accepted_by_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_repository_invites
    ADD CONSTRAINT fk_scope_repository_invites_inviter FOREIGN KEY (invited_by_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_invites
    ADD CONSTRAINT fk_scope_repository_invites_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_members
    ADD CONSTRAINT fk_scope_repository_members_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_members
    ADD CONSTRAINT fk_scope_repository_members_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussion_read_states
    ADD CONSTRAINT fk_scope_request_discussion_read_states_discussion FOREIGN KEY (discussion_id) REFERENCES scope_request_discussions(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussion_read_states
    ADD CONSTRAINT fk_scope_request_discussion_read_states_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT fk_scope_request_discussion_replies_author FOREIGN KEY (author_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT fk_scope_request_discussion_replies_discussion FOREIGN KEY (discussion_id) REFERENCES scope_request_discussions(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT fk_scope_request_discussion_replies_quoted_reply FOREIGN KEY (reply_to_reply_id) REFERENCES scope_request_discussion_replies(id) ON DELETE RESTRICT;

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_author FOREIGN KEY (author_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_resolver FOREIGN KEY (resolved_by_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_revision FOREIGN KEY (request_id, revision_id) REFERENCES scope_request_revisions(request_id, id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_events
    ADD CONSTRAINT fk_scope_request_events_actor FOREIGN KEY (actor_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_events
    ADD CONSTRAINT fk_scope_request_events_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT fk_scope_request_invitees_inviter FOREIGN KEY (invited_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;

ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT fk_scope_request_invitees_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT fk_scope_request_invitees_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_ratings
    ADD CONSTRAINT fk_scope_request_ratings_rater FOREIGN KEY (rater_user_id) REFERENCES scope_users(id);

ALTER TABLE ONLY scope_request_ratings
    ADD CONSTRAINT fk_scope_request_ratings_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_ratings
    ADD CONSTRAINT fk_scope_request_ratings_subject FOREIGN KEY (subject_user_id) REFERENCES scope_users(id);

ALTER TABLE ONLY scope_request_revisions
    ADD CONSTRAINT fk_scope_request_revisions_actor FOREIGN KEY (actor_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_revisions
    ADD CONSTRAINT fk_scope_request_revisions_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_author FOREIGN KEY (author_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_closer FOREIGN KEY (closed_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_merger FOREIGN KEY (merged_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_run_attempt_steps
    ADD CONSTRAINT fk_scope_run_attempt_steps_attempt FOREIGN KEY (attempt_id) REFERENCES scope_run_attempts(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_run_attempts
    ADD CONSTRAINT fk_scope_run_attempts_job FOREIGN KEY (run_id, job_key) REFERENCES scope_run_jobs(run_id, job_key) ON DELETE CASCADE;

ALTER TABLE ONLY scope_run_attempts
    ADD CONSTRAINT fk_scope_run_attempts_run FOREIGN KEY (run_id) REFERENCES scope_runs(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_run_jobs
    ADD CONSTRAINT fk_scope_run_jobs_current_attempt FOREIGN KEY (current_attempt_id, run_id, job_key) REFERENCES scope_run_attempts(id, run_id, job_key) ON DELETE SET NULL (current_attempt_id);

ALTER TABLE ONLY scope_run_jobs
    ADD CONSTRAINT fk_scope_run_jobs_run FOREIGN KEY (run_id) REFERENCES scope_runs(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_run_logs
    ADD CONSTRAINT fk_scope_run_logs_run FOREIGN KEY (run_id) REFERENCES scope_runs(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_run_logs
    ADD CONSTRAINT fk_scope_run_logs_step FOREIGN KEY (attempt_id, step_index) REFERENCES scope_run_attempt_steps(attempt_id, step_index) ON DELETE CASCADE;

ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT fk_scope_runs_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT fk_scope_runs_requester FOREIGN KEY (requested_by_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;

ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT fk_scope_runs_revision FOREIGN KEY (workflow_revision_digest) REFERENCES scope_workflow_revisions(digest) ON DELETE RESTRICT;

ALTER TABLE ONLY scope_cache_objects
    ADD CONSTRAINT scope_cache_objects_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_cache_orphan_uploads
    ADD CONSTRAINT scope_cache_orphan_uploads_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_cache_uploads
    ADD CONSTRAINT scope_cache_uploads_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_git_compaction_jobs
    ADD CONSTRAINT scope_git_compaction_jobs_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_git_segment_references
    ADD CONSTRAINT scope_git_segment_references_segment_id_fkey FOREIGN KEY (segment_id) REFERENCES scope_git_segment_uploads(segment_id) ON DELETE RESTRICT;

ALTER TABLE ONLY scope_repository_history_entries
    ADD CONSTRAINT scope_repository_history_entries_repo_id_audience_fkey FOREIGN KEY (repo_id, audience) REFERENCES scope_repository_history_views(repo_id, audience) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_history_views
    ADD CONSTRAINT scope_repository_history_views_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_landing_files
    ADD CONSTRAINT scope_repository_landing_files_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_workflow_catalogs
    ADD CONSTRAINT scope_repository_workflow_catalogs_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_workflow_files
    ADD CONSTRAINT scope_repository_workflow_files_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repository_workflow_catalogs(repo_id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_media_abandoned_objects
    ADD CONSTRAINT scope_request_media_abandoned_objects_attachment_id_fkey FOREIGN KEY (attachment_id) REFERENCES scope_request_media_attachments(id);

ALTER TABLE ONLY scope_request_media_bindings
    ADD CONSTRAINT scope_request_media_bindings_attachment_id_fkey FOREIGN KEY (attachment_id) REFERENCES scope_request_media_attachments(id);

ALTER TABLE ONLY scope_request_media_cleanup_jobs
    ADD CONSTRAINT scope_request_media_cleanup_jobs_attachment_id_fkey FOREIGN KEY (attachment_id) REFERENCES scope_request_media_attachments(id);

ALTER TABLE ONLY scope_request_media_derivatives
    ADD CONSTRAINT scope_request_media_derivatives_attachment_id_fkey FOREIGN KEY (attachment_id) REFERENCES scope_request_media_attachments(id);

ALTER TABLE ONLY scope_request_media_derivatives
    ADD CONSTRAINT scope_request_media_derivatives_manifest_id_fkey FOREIGN KEY (manifest_id) REFERENCES scope_request_media_manifests(id);

ALTER TABLE ONLY scope_request_media_manifest_chunks
    ADD CONSTRAINT scope_request_media_manifest_chunks_manifest_id_fkey FOREIGN KEY (manifest_id) REFERENCES scope_request_media_manifests(id);

ALTER TABLE ONLY scope_request_media_manifests
    ADD CONSTRAINT scope_request_media_manifests_attachment_id_fkey FOREIGN KEY (attachment_id) REFERENCES scope_request_media_attachments(id);

ALTER TABLE ONLY scope_request_media_orphan_cleanup_leases
    ADD CONSTRAINT scope_request_media_orphan_cleanup_leases_attachment_id_fkey FOREIGN KEY (attachment_id) REFERENCES scope_request_media_attachments(id);

ALTER TABLE ONLY scope_request_media_processing_jobs
    ADD CONSTRAINT scope_request_media_processing_jobs_attachment_id_fkey FOREIGN KEY (attachment_id) REFERENCES scope_request_media_attachments(id);

ALTER TABLE ONLY scope_request_media_processing_objects
    ADD CONSTRAINT scope_request_media_processing_objects_attachment_id_fkey FOREIGN KEY (attachment_id) REFERENCES scope_request_media_attachments(id);

ALTER TABLE ONLY scope_request_media_retry_operations
    ADD CONSTRAINT scope_request_media_retry_operations_attachment_id_fkey FOREIGN KEY (attachment_id) REFERENCES scope_request_media_attachments(id);

ALTER TABLE ONLY scope_request_media_upload_parts
    ADD CONSTRAINT scope_request_media_upload_parts_attachment_id_fkey FOREIGN KEY (attachment_id) REFERENCES scope_request_media_attachments(id);

ALTER TABLE ONLY scope_run_attempt_cache_setups
    ADD CONSTRAINT scope_run_attempt_cache_setups_attempt_id_fkey FOREIGN KEY (attempt_id) REFERENCES scope_run_attempts(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_run_attempt_caches
    ADD CONSTRAINT scope_run_attempt_caches_attempt_id_fkey FOREIGN KEY (attempt_id) REFERENCES scope_run_attempts(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_visibility_change_sets
    ADD CONSTRAINT scope_visibility_change_sets_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_visibility_changes
    ADD CONSTRAINT scope_visibility_changes_repo_id_change_set_id_fkey FOREIGN KEY (repo_id, change_set_id) REFERENCES scope_visibility_change_sets(repo_id, id) ON DELETE CASCADE;
