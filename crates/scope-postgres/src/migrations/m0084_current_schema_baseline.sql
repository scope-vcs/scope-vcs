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
    cleanup_generation bigint DEFAULT 0 NOT NULL,
    cleanup_lease_expires_at_unix bigint,
    CONSTRAINT scope_cache_upload_cleanup_values CHECK (((cleanup_generation >= 0) AND (((state = 'deleting'::text) AND (cleanup_lease_expires_at_unix IS NOT NULL) AND (cleanup_lease_expires_at_unix >= 0)) OR ((state <> 'deleting'::text) AND (cleanup_lease_expires_at_unix IS NULL))))),
    CONSTRAINT scope_cache_uploads_values CHECK (((char_length(upload_id) >= 1) AND (char_length(upload_id) <= 128) AND (upload_id !~ '[[:space:]]'::text) AND ((identity_digest)::text ~ '^[0-9a-f]{64}$'::text) AND ((compatibility_group_digest)::text ~ '^[0-9a-f]{64}$'::text) AND ((checksum_sha256)::text ~ '^[0-9a-f]{64}$'::text) AND ((char_length((storage_backend)::text) >= 1) AND (char_length((storage_backend)::text) <= 64)) AND ((storage_backend)::text ~ '^[a-z0-9]+(-[a-z0-9]+)*$'::text) AND (object_key = ((('repos/'::text || repository_id) || '/objects/sha256/'::text) || (checksum_sha256)::text)) AND ((size_bytes >= 1) AND (size_bytes <= 1073741824)) AND (state = ANY (ARRAY['active'::text, 'deleting'::text, 'committed'::text])) AND (created_at_unix >= 0) AND (expires_at_unix > created_at_unix) AND (expires_at_unix <= (created_at_unix + 1800))))
);

CREATE TABLE scope_clerk_user_deletions (
    clerk_user_id character varying NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    next_attempt_at_unix bigint NOT NULL,
    claim_token character varying,
    claim_expires_at_unix bigint,
    last_error text,
    created_at_unix bigint NOT NULL,
    completed_at_unix bigint,
    CONSTRAINT scope_clerk_user_deletion_values CHECK (((length(btrim((clerk_user_id)::text)) > 0) AND (attempts >= 0) AND (next_attempt_at_unix >= 0) AND ((claim_token IS NULL) = (claim_expires_at_unix IS NULL)) AND ((completed_at_unix IS NULL) OR ((completed_at_unix >= 0) AND (claim_token IS NULL))) AND ((last_error IS NULL) OR ((octet_length(last_error) >= 1) AND (octet_length(last_error) <= 8192))) AND (created_at_unix >= 0)))
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

CREATE TABLE scope_dependency_analyses (
    repo_id text NOT NULL,
    incarnation_id text NOT NULL,
    head_oid text NOT NULL,
    analyzer_version text NOT NULL,
    analysis jsonb NOT NULL,
    completed_at_unix bigint NOT NULL,
    CONSTRAINT scope_dependency_analyses_values CHECK (((length(btrim(incarnation_id)) > 0) AND (length(btrim(head_oid)) > 0) AND ((length(btrim(analyzer_version)) >= 1) AND (length(btrim(analyzer_version)) <= 200)) AND (jsonb_typeof(analysis) = 'object'::text) AND (completed_at_unix >= 0)))
);

CREATE TABLE scope_dependency_analysis_jobs (
    repo_id text NOT NULL,
    incarnation_id text NOT NULL,
    repo_version bigint NOT NULL,
    head_oid text NOT NULL,
    analyzer_version text NOT NULL,
    attempts integer NOT NULL,
    next_run_at_unix bigint NOT NULL,
    lease_generation text,
    lease_owner text,
    lease_expires_at_unix bigint,
    last_error text,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_dependency_analysis_jobs_values CHECK (((length(btrim(incarnation_id)) > 0) AND (repo_version > 0) AND (length(btrim(head_oid)) > 0) AND ((length(btrim(analyzer_version)) >= 1) AND (length(btrim(analyzer_version)) <= 200)) AND (attempts >= 0) AND (next_run_at_unix >= 0) AND ((lease_generation IS NULL) = (lease_owner IS NULL)) AND ((lease_generation IS NULL) = (lease_expires_at_unix IS NULL)) AND ((lease_generation IS NULL) OR (length(btrim(lease_generation)) > 0)) AND ((lease_owner IS NULL) OR (length(btrim(lease_owner)) > 0)) AND ((lease_expires_at_unix IS NULL) OR (lease_expires_at_unix >= 0)) AND ((last_error IS NULL) OR ((length(last_error) >= 1) AND (length(last_error) <= 2000))) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix)))
);

CREATE TABLE scope_dependency_reports (
    repo_id text NOT NULL,
    incarnation_id text NOT NULL,
    repo_version bigint NOT NULL,
    head_oid text NOT NULL,
    analyzer_version text NOT NULL,
    report jsonb NOT NULL,
    completed_at_unix bigint NOT NULL,
    CONSTRAINT scope_dependency_reports_values CHECK (((length(btrim(incarnation_id)) > 0) AND (repo_version > 0) AND (length(btrim(head_oid)) > 0) AND ((length(btrim(analyzer_version)) >= 1) AND (length(btrim(analyzer_version)) <= 200)) AND (jsonb_typeof(report) = 'object'::text) AND (completed_at_unix >= 0)))
);

CREATE TABLE scope_file_changes (
    repo_id character varying NOT NULL,
    commit_id character varying NOT NULL,
    ordinal bigint NOT NULL,
    path text NOT NULL,
    old_content jsonb,
    new_content jsonb,
    visibility character varying NOT NULL,
    CONSTRAINT scope_file_change_ordinal CHECK ((ordinal >= 0)),
    CONSTRAINT scope_file_change_view CHECK ((length(btrim((visibility)::text)) > 0))
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
    frontier_digest character varying CONSTRAINT scope_git_heads_manifest_sha256_not_null NOT NULL,
    CONSTRAINT scope_git_head_values CHECK (((push_sequence >= 0) AND (change_version >= 0)))
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

CREATE TABLE scope_github_check_refreshes (
    repo_id character varying NOT NULL,
    github_repository_id bigint NOT NULL,
    commit_oid character varying NOT NULL,
    next_read_at_unix bigint NOT NULL,
    started_reads bigint NOT NULL,
    applied_read bigint NOT NULL,
    applied_read_started_at_unix bigint,
    CONSTRAINT scope_github_check_refresh_values CHECK (((github_repository_id > 0) AND (length((commit_oid)::text) = 40) AND (next_read_at_unix >= 0) AND (applied_read >= 0) AND (started_reads >= applied_read) AND ((applied_read = 0) = (applied_read_started_at_unix IS NULL))))
);

CREATE TABLE scope_github_check_runs (
    github_check_run_id bigint NOT NULL,
    repo_id character varying NOT NULL,
    commit_oid character varying NOT NULL,
    name text NOT NULL,
    status character varying NOT NULL,
    conclusion character varying,
    details_url text,
    updated_at_unix bigint NOT NULL,
    github_repository_id bigint NOT NULL,
    check_suite_id bigint,
    CONSTRAINT scope_github_check_run_repository CHECK ((github_repository_id > 0)),
    CONSTRAINT scope_github_check_run_suite CHECK (((check_suite_id IS NULL) OR (check_suite_id > 0))),
    CONSTRAINT scope_github_check_run_values CHECK (((github_check_run_id > 0) AND (length((commit_oid)::text) = 40) AND (length(btrim(name)) > 0) AND ((status)::text = ANY ((ARRAY['queued'::character varying, 'in_progress'::character varying, 'completed'::character varying, 'waiting'::character varying, 'requested'::character varying, 'pending'::character varying])::text[])) AND ((conclusion)::text = ANY ((ARRAY['success'::character varying, 'neutral'::character varying, 'skipped'::character varying, 'failure'::character varying, 'cancelled'::character varying, 'timed_out'::character varying, 'action_required'::character varying, 'stale'::character varying, 'startup_failure'::character varying])::text[])) AND (((status)::text = 'completed'::text) = (conclusion IS NOT NULL)) AND (updated_at_unix >= 0)))
);

CREATE TABLE scope_github_connections (
    repo_id character varying NOT NULL,
    installation_id bigint NOT NULL,
    github_repository_id bigint NOT NULL,
    github_full_name character varying NOT NULL,
    connected_by_user_id character varying,
    connected_at_unix bigint NOT NULL,
    status character varying NOT NULL,
    disconnect_reason character varying,
    disconnected_at_unix bigint,
    github_visibility character varying DEFAULT 'Private'::character varying NOT NULL,
    CONSTRAINT scope_github_connection_values CHECK (((installation_id > 0) AND (github_repository_id > 0) AND (length(btrim((github_full_name)::text)) > 0) AND (connected_at_unix >= 0) AND ((status)::text = ANY ((ARRAY['Connected'::character varying, 'Disconnected'::character varying])::text[])) AND (((status)::text = 'Connected'::text) = (disconnect_reason IS NULL)) AND ((disconnect_reason IS NULL) = (disconnected_at_unix IS NULL)) AND ((disconnect_reason IS NULL) OR ((disconnect_reason)::text = ANY ((ARRAY['AppUninstalled'::character varying, 'InstallationSuspended'::character varying, 'RepositoryRemoved'::character varying])::text[]))) AND ((disconnected_at_unix IS NULL) OR (disconnected_at_unix >= connected_at_unix)))),
    CONSTRAINT scope_github_connection_visibility CHECK (((github_visibility)::text = ANY ((ARRAY['Private'::character varying, 'PublicUnacknowledged'::character varying, 'PublicAcknowledged'::character varying])::text[])))
);

CREATE TABLE scope_github_pushes (
    id character varying NOT NULL,
    repo_id character varying NOT NULL,
    request_id character varying,
    ref character varying NOT NULL,
    sequence bigint NOT NULL,
    installation_id bigint NOT NULL,
    github_repository_id bigint NOT NULL,
    github_full_name character varying NOT NULL,
    target_oid character varying,
    kind character varying NOT NULL,
    state character varying NOT NULL,
    attempts integer NOT NULL,
    next_attempt_at_unix bigint NOT NULL,
    lease_until_unix bigint,
    claim_token character varying,
    last_error text,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_github_push_values CHECK (((((request_id IS NOT NULL) AND ((ref)::text = ('refs/heads/scope/requests/'::text || (request_id)::text))) OR ((request_id IS NULL) AND ((ref)::text = 'refs/heads/scope/setup-check'::text))) AND (sequence > 0) AND (installation_id > 0) AND (github_repository_id > 0) AND (length(btrim((github_full_name)::text)) > 0) AND ((kind)::text = ANY ((ARRAY['push'::character varying, 'delete'::character varying])::text[])) AND (((kind)::text = 'push'::text) = (target_oid IS NOT NULL)) AND ((target_oid IS NULL) OR (length((target_oid)::text) = 40)) AND ((state)::text = ANY ((ARRAY['queued'::character varying, 'running'::character varying, 'succeeded'::character varying, 'failed'::character varying])::text[])) AND (((state)::text = 'running'::text) = (lease_until_unix IS NOT NULL)) AND (((state)::text = 'running'::text) = (claim_token IS NOT NULL)) AND (((state)::text <> 'failed'::text) OR (last_error IS NOT NULL)) AND (attempts >= 0) AND (next_attempt_at_unix >= 0) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix)))
);

CREATE TABLE scope_github_required_checks (
    repo_id character varying NOT NULL,
    name text NOT NULL,
    "position" integer NOT NULL,
    CONSTRAINT scope_github_required_check_values CHECK (((length(btrim(name)) > 0) AND (name = btrim(name)) AND (char_length(name) <= 255) AND ("position" >= 0)))
);

CREATE TABLE scope_github_run_import_counts (
    repo_id character varying NOT NULL,
    run_count integer NOT NULL,
    CONSTRAINT scope_github_run_import_count_values CHECK (((run_count >= 0) AND (run_count <= 1000)))
);

CREATE TABLE scope_github_run_imports (
    repo_id character varying NOT NULL,
    github_repository_id bigint NOT NULL,
    run_count integer NOT NULL,
    state character varying NOT NULL,
    attempts integer NOT NULL,
    imported_count integer NOT NULL,
    last_error text,
    next_attempt_at_unix bigint NOT NULL,
    lease_until_unix bigint,
    claim_token character varying,
    queued_at_unix bigint NOT NULL,
    finished_at_unix bigint,
    CONSTRAINT scope_github_run_import_values CHECK (((github_repository_id > 0) AND ((run_count >= 1) AND (run_count <= 1000)) AND ((state)::text = ANY ((ARRAY['queued'::character varying, 'running'::character varying, 'succeeded'::character varying, 'failed'::character varying])::text[])) AND (((state)::text = 'running'::text) = (lease_until_unix IS NOT NULL)) AND (((state)::text = 'running'::text) = (claim_token IS NOT NULL)) AND (((state)::text = ANY ((ARRAY['succeeded'::character varying, 'failed'::character varying])::text[])) = (finished_at_unix IS NOT NULL)) AND (((state)::text <> 'failed'::text) OR (last_error IS NOT NULL)) AND (((state)::text <> 'succeeded'::text) OR (last_error IS NULL)) AND (attempts >= 0) AND ((imported_count >= 0) AND (imported_count <= run_count)) AND (next_attempt_at_unix >= 0) AND (queued_at_unix >= 0) AND ((finished_at_unix IS NULL) OR (finished_at_unix >= queued_at_unix))))
);

CREATE TABLE scope_github_setup_checks (
    repo_id character varying NOT NULL,
    github_repository_id bigint NOT NULL,
    commit_oid character varying NOT NULL,
    state character varying NOT NULL,
    started_at_unix bigint NOT NULL,
    finished_at_unix bigint,
    last_error text,
    push_id character varying,
    baseline_run_ids jsonb,
    CONSTRAINT scope_github_setup_check_values CHECK (((github_repository_id > 0) AND (length((commit_oid)::text) = 40) AND ((state)::text = ANY ((ARRAY['pushing'::character varying, 'waiting'::character varying, 'finished'::character varying, 'failed'::character varying])::text[])) AND (((state)::text = ANY ((ARRAY['finished'::character varying, 'failed'::character varying])::text[])) = (finished_at_unix IS NOT NULL)) AND (((state)::text = 'failed'::text) = (last_error IS NOT NULL)) AND (started_at_unix >= 0) AND ((finished_at_unix IS NULL) OR (finished_at_unix >= started_at_unix)) AND ((baseline_run_ids IS NULL) OR (jsonb_typeof(baseline_run_ids) = 'array'::text))))
);

CREATE TABLE scope_github_workflow_job_log_reads (
    github_job_id bigint NOT NULL,
    attempts integer NOT NULL,
    next_attempt_at_unix bigint CONSTRAINT scope_github_workflow_job_log_rea_next_attempt_at_unix_not_null NOT NULL,
    CONSTRAINT scope_github_workflow_job_log_read_values CHECK (((attempts >= 0) AND (next_attempt_at_unix >= 0)))
);

CREATE TABLE scope_github_workflow_job_logs (
    github_job_id bigint NOT NULL,
    log_text text,
    truncated boolean NOT NULL,
    stored_at_unix bigint NOT NULL,
    CONSTRAINT scope_github_workflow_job_log_values CHECK (((stored_at_unix >= 0) AND ((log_text IS NOT NULL) OR (NOT truncated))))
);

CREATE TABLE scope_github_workflow_job_reads (
    read_id bigint NOT NULL,
    repo_id character varying NOT NULL,
    github_repository_id bigint NOT NULL,
    github_run_id bigint NOT NULL,
    attempts integer NOT NULL,
    generation bigint NOT NULL,
    next_attempt_at_unix bigint NOT NULL,
    CONSTRAINT scope_github_workflow_job_read_values CHECK (((github_repository_id > 0) AND (github_run_id > 0) AND (attempts >= 0) AND (generation >= 0) AND (next_attempt_at_unix >= 0)))
);

ALTER TABLE scope_github_workflow_job_reads ALTER COLUMN read_id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME scope_github_workflow_job_reads_read_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);

CREATE TABLE scope_github_workflow_jobs (
    github_job_id bigint NOT NULL,
    repo_id character varying NOT NULL,
    github_repository_id bigint NOT NULL,
    github_run_id bigint NOT NULL,
    run_attempt integer NOT NULL,
    name text NOT NULL,
    status character varying NOT NULL,
    conclusion character varying,
    started_at_unix bigint,
    completed_at_unix bigint,
    html_url text NOT NULL,
    steps jsonb NOT NULL,
    stage smallint NOT NULL,
    steps_completed integer NOT NULL,
    steps_started integer NOT NULL,
    CONSTRAINT scope_github_workflow_job_values CHECK (((github_job_id > 0) AND (github_repository_id > 0) AND (github_run_id > 0) AND (run_attempt > 0) AND (length(btrim(name)) > 0) AND ((status)::text = ANY ((ARRAY['queued'::character varying, 'in_progress'::character varying, 'completed'::character varying, 'waiting'::character varying, 'requested'::character varying, 'pending'::character varying])::text[])) AND ((conclusion)::text = ANY ((ARRAY['success'::character varying, 'neutral'::character varying, 'skipped'::character varying, 'failure'::character varying, 'cancelled'::character varying, 'timed_out'::character varying, 'action_required'::character varying, 'stale'::character varying, 'startup_failure'::character varying])::text[])) AND (((status)::text = 'completed'::text) = (conclusion IS NOT NULL)) AND ((started_at_unix IS NULL) OR (started_at_unix >= 0)) AND ((completed_at_unix IS NULL) OR (completed_at_unix >= 0)) AND (jsonb_typeof(steps) = 'array'::text) AND (stage = ANY (ARRAY[0, 1, 2])) AND ((stage = 2) = ((status)::text = 'completed'::text)) AND (steps_completed >= 0) AND (steps_started >= steps_completed)))
);

CREATE TABLE scope_github_workflow_run_reads (
    repo_id character varying NOT NULL,
    github_repository_id bigint NOT NULL,
    github_run_id bigint NOT NULL,
    attempts integer NOT NULL,
    next_attempt_at_unix bigint NOT NULL,
    CONSTRAINT scope_github_workflow_run_read_values CHECK (((github_repository_id > 0) AND (github_run_id > 0) AND (attempts >= 0) AND (next_attempt_at_unix >= 0)))
);

CREATE TABLE scope_github_workflow_runs (
    github_run_id bigint NOT NULL,
    repo_id character varying NOT NULL,
    github_repository_id bigint NOT NULL,
    workflow_name text NOT NULL,
    head_branch text,
    head_oid character varying NOT NULL,
    event character varying NOT NULL,
    status character varying NOT NULL,
    conclusion character varying,
    html_url text NOT NULL,
    check_suite_id bigint,
    run_started_at_unix bigint,
    run_attempt integer NOT NULL,
    stage smallint NOT NULL,
    github_updated_at_unix bigint NOT NULL,
    jobs_read_attempt integer,
    jobs_read_at_unix bigint,
    request_id text,
    CONSTRAINT scope_github_workflow_run_jobs_read CHECK ((((jobs_read_attempt IS NULL) = (jobs_read_at_unix IS NULL)) AND ((jobs_read_attempt IS NULL) OR (jobs_read_attempt > 0)) AND ((jobs_read_at_unix IS NULL) OR (jobs_read_at_unix >= 0)))),
    CONSTRAINT scope_github_workflow_run_values CHECK (((github_run_id > 0) AND (github_repository_id > 0) AND (length(btrim(workflow_name)) > 0) AND (length((head_oid)::text) = 40) AND (length(btrim((event)::text)) > 0) AND ((status)::text = ANY ((ARRAY['queued'::character varying, 'in_progress'::character varying, 'completed'::character varying, 'waiting'::character varying, 'requested'::character varying, 'pending'::character varying])::text[])) AND ((conclusion)::text = ANY ((ARRAY['success'::character varying, 'neutral'::character varying, 'skipped'::character varying, 'failure'::character varying, 'cancelled'::character varying, 'timed_out'::character varying, 'action_required'::character varying, 'stale'::character varying, 'startup_failure'::character varying])::text[])) AND (((status)::text = 'completed'::text) = (conclusion IS NOT NULL)) AND ((check_suite_id IS NULL) OR (check_suite_id > 0)) AND ((run_started_at_unix IS NULL) OR (run_started_at_unix >= 0)) AND (run_attempt > 0) AND (stage = ANY (ARRAY[0, 1, 2])) AND ((stage = 2) = ((status)::text = 'completed'::text)) AND (github_updated_at_unix >= 0)))
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

CREATE TABLE scope_native_runs_accounts (
    user_id character varying NOT NULL,
    added_at_unix bigint NOT NULL,
    note text,
    CONSTRAINT scope_native_runs_account_values CHECK (((added_at_unix >= 0) AND ((note IS NULL) OR ((length(btrim(note)) > 0) AND (char_length(note) <= 500)))))
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
    CONSTRAINT scope_outbox_values CHECK (((repo_version >= 0) AND (attempts >= 0) AND (next_run_at_unix >= 0) AND (created_at_unix >= 0) AND (updated_at_unix >= 0) AND ((state)::text = ANY (ARRAY[('ready'::character varying)::text, ('running'::character varying)::text, ('succeeded'::character varying)::text, ('failed'::character varying)::text])) AND ((lease_expires_at_unix IS NULL) OR (lease_expires_at_unix >= 0)) AND ((completed_at_unix IS NULL) OR (completed_at_unix >= 0))))
);

CREATE TABLE scope_projection_files (
    repo_id character varying NOT NULL,
    path_key character varying NOT NULL,
    path character varying NOT NULL,
    oid character varying NOT NULL,
    visibility character varying NOT NULL,
    object_key character varying NOT NULL,
    sha256 character varying NOT NULL,
    size_bytes bigint NOT NULL,
    git_file_mode character varying NOT NULL,
    CONSTRAINT scope_projection_file_values CHECK (((length(btrim((visibility)::text)) > 0) AND (size_bytes >= 0) AND ((git_file_mode)::text = ANY ((ARRAY['100644'::character varying, '100755'::character varying])::text[]))))
);

CREATE TABLE scope_projection_read_models (
    repo_id character varying NOT NULL,
    audience character varying NOT NULL,
    repo_version bigint NOT NULL,
    identity_version smallint NOT NULL,
    history_version text NOT NULL,
    folded_commits bigint NOT NULL,
    folded_change_sets bigint NOT NULL,
    last_commit_id character varying,
    last_change_set_id character varying,
    projected_commits bigint NOT NULL,
    last_projected_id character varying,
    head_oid character varying,
    file_count bigint NOT NULL,
    visible_files boolean NOT NULL,
    history_entries bigint NOT NULL,
    last_history_entry_id text,
    history_generation text NOT NULL,
    views jsonb NOT NULL,
    CONSTRAINT scope_projection_read_model_values CHECK (((length(btrim((audience)::text)) > 0) AND (repo_version >= 0) AND (folded_commits >= 0) AND (folded_change_sets >= 0) AND ((last_commit_id IS NULL) = (folded_commits = 0)) AND ((last_change_set_id IS NULL) = (folded_change_sets = 0)) AND (projected_commits >= 0) AND ((last_projected_id IS NULL) = (projected_commits = 0)) AND ((head_oid IS NULL) = (projected_commits = 0)) AND ((head_oid IS NULL) OR ((head_oid)::text ~ '^[0-9a-f]{40}$'::text)) AND (file_count >= 0) AND (history_entries >= 0) AND ((last_history_entry_id IS NULL) = (history_entries = 0)))),
    CONSTRAINT scope_projection_read_model_views CHECK (((jsonb_typeof(views) = 'array'::text) AND (jsonb_array_length(views) > 0)))
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
    CONSTRAINT scope_push_trigger_evaluation_values CHECK (((change_version > 0) AND (length((head_oid)::text) = 40) AND (created_at_unix >= 0) AND ((state)::text = ANY (ARRAY[('pending'::character varying)::text, ('succeeded'::character varying)::text, ('configuration-error'::character varying)::text, ('failed'::character varying)::text])) AND ((((state)::text = 'pending'::text) AND (message IS NULL) AND (completed_at_unix IS NULL)) OR (((state)::text = 'succeeded'::text) AND (message IS NULL) AND (completed_at_unix IS NOT NULL)) OR (((state)::text = ANY (ARRAY[('configuration-error'::character varying)::text, ('failed'::character varying)::text])) AND (length(btrim(message)) > 0) AND (completed_at_unix IS NOT NULL)))))
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
    content_version bigint NOT NULL,
    CONSTRAINT scope_repositories_content_version_bounds CHECK (((content_version >= 0) AND (content_version <= change_version))),
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
    repo_id character varying NOT NULL,
    audience character varying NOT NULL,
    "position" bigint NOT NULL,
    source_id text NOT NULL,
    payload_hash text NOT NULL,
    CONSTRAINT scope_repository_history_entries_position_check CHECK (("position" >= 0))
);

CREATE TABLE scope_repository_history_payloads (
    repo_id character varying NOT NULL,
    payload_hash text NOT NULL,
    payload jsonb NOT NULL
);

CREATE TABLE scope_repository_invite_emails (
    id character varying NOT NULL,
    invite_id character varying,
    requested_by_user_id character varying NOT NULL,
    state character varying NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    next_attempt_at_unix bigint NOT NULL,
    claim_token character varying,
    claim_expires_at_unix bigint,
    provider_message_id character varying,
    last_error text,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_repository_invite_email_values CHECK (((length(btrim((id)::text)) > 0) AND ((state)::text = ANY ((ARRAY['Queued'::character varying, 'Sent'::character varying, 'Failed'::character varying])::text[])) AND (attempts >= 0) AND (next_attempt_at_unix >= 0) AND ((claim_token IS NULL) = (claim_expires_at_unix IS NULL)) AND (((state)::text = 'Queued'::text) OR (claim_token IS NULL)) AND ((last_error IS NULL) OR ((octet_length(last_error) >= 1) AND (octet_length(last_error) <= 8192))) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix)))
);

CREATE TABLE scope_repository_invite_links (
    token_hash character varying NOT NULL,
    invite_id character varying NOT NULL,
    CONSTRAINT scope_repository_invite_link_values CHECK ((length(btrim((token_hash)::text)) > 0))
);

CREATE TABLE scope_repository_invites (
    id character varying NOT NULL,
    repo_id character varying NOT NULL,
    invited_email character varying NOT NULL,
    invited_email_normalized character varying NOT NULL,
    permissions jsonb NOT NULL,
    invited_by_user_id character varying NOT NULL,
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
    source_change_version bigint CONSTRAINT scope_repository_workflow_catalo_source_change_version_not_null NOT NULL,
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

CREATE TABLE scope_request_attention_states (
    request_id character varying NOT NULL,
    user_id character varying NOT NULL,
    state character varying NOT NULL,
    reason character varying NOT NULL,
    through_activity_version bigint CONSTRAINT scope_request_attention_state_through_activity_version_not_null NOT NULL,
    snoozed_until_unix bigint,
    updated_at_unix bigint NOT NULL,
    revision bigint NOT NULL,
    CONSTRAINT scope_request_attention_states_revision_check CHECK ((revision >= 1)),
    CONSTRAINT scope_request_attention_values CHECK ((((state)::text = ANY ((ARRAY['active'::character varying, 'waiting'::character varying, 'snoozed'::character varying, 'settled'::character varying])::text[])) AND ((reason)::text = ANY ((ARRAY['claimed'::character varying, 'new_activity'::character varying, 'restored'::character varying, 'waiting'::character varying, 'snoozed'::character varying, 'settled'::character varying])::text[])) AND (through_activity_version >= 0) AND (updated_at_unix >= 0) AND (((state)::text = 'snoozed'::text) = (snoozed_until_unix IS NOT NULL)) AND ((snoozed_until_unix IS NULL) OR (snoozed_until_unix > updated_at_unix))))
);

CREATE TABLE scope_request_auto_merge_intents (
    id character varying NOT NULL,
    repo_id character varying NOT NULL,
    repository_incarnation_id text CONSTRAINT scope_request_auto_merge_int_repository_incarnation_id_not_null NOT NULL,
    request_id character varying NOT NULL,
    revision_id character varying NOT NULL,
    head_oid character varying NOT NULL,
    actor_user_id character varying NOT NULL,
    status character varying NOT NULL,
    reason character varying,
    created_position bigint NOT NULL,
    claim_token character varying,
    claim_expires_at_unix bigint,
    attempt integer DEFAULT 0 NOT NULL,
    next_attempt_at_unix bigint NOT NULL,
    last_error text,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_auto_merge_values CHECK (((length(btrim((id)::text)) > 0) AND (length(btrim((repo_id)::text)) > 0) AND (length(btrim(repository_incarnation_id)) > 0) AND (length(btrim((request_id)::text)) > 0) AND (length(btrim((revision_id)::text)) > 0) AND (length((head_oid)::text) = 40) AND (length(btrim((actor_user_id)::text)) > 0) AND ((status)::text = ANY ((ARRAY['Active'::character varying, 'Cancelled'::character varying, 'Stopped'::character varying, 'Fulfilled'::character varying])::text[])) AND ((reason)::text = ANY ((ARRAY['RequestChanged'::character varying, 'RequestClosed'::character varying, 'AccessRevoked'::character varying, 'ChecksFailed'::character varying, 'ChecksConfigurationError'::character varying, 'MergeConflict'::character varying, 'RequestBranchMissing'::character varying])::text[])) AND (((status)::text = 'Stopped'::text) = (reason IS NOT NULL)) AND (created_position > 0) AND ((claim_token IS NULL) = (claim_expires_at_unix IS NULL)) AND (((status)::text = 'Active'::text) OR (claim_token IS NULL)) AND (attempt >= 0) AND (next_attempt_at_unix >= 0) AND ((last_error IS NULL) OR ((octet_length(last_error) >= 1) AND (octet_length(last_error) <= 8192))) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix)))
);

CREATE TABLE scope_request_check_evaluations (
    request_id character varying NOT NULL,
    head_oid character varying NOT NULL,
    state character varying NOT NULL,
    message text,
    checks jsonb NOT NULL,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    tested_oid character varying NOT NULL,
    check_private_main_oid character varying,
    check_public_base_oid character varying,
    changes_github_workflows boolean,
    CONSTRAINT scope_request_check_evaluation_check_commit CHECK ((((check_private_main_oid IS NULL) = (check_public_base_oid IS NULL)) AND ((check_private_main_oid IS NULL) = ((tested_oid)::text = (head_oid)::text)) AND ((check_private_main_oid IS NULL) OR ((length((check_private_main_oid)::text) = 40) AND (length((check_public_base_oid)::text) = 40))))),
    CONSTRAINT scope_request_check_evaluation_values CHECK (((length((head_oid)::text) = 40) AND (length((tested_oid)::text) = 40) AND ((state)::text = ANY ((ARRAY['no-checks'::character varying, 'awaiting-approval'::character varying, 'started'::character varying, 'configuration-error'::character varying])::text[])) AND (jsonb_typeof(checks) = 'array'::text) AND (NOT (checks @? '$[*]?(!(@."provider" == "native" || @."provider" == "github"))'::jsonpath)) AND (((tested_oid)::text = (head_oid)::text) OR (NOT (checks @? '$[*]?(@."provider" == "native")'::jsonpath))) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix) AND (((state)::text = 'configuration-error'::text) = (length(btrim(COALESCE(message, ''::text))) > 0)) AND (((state)::text = ANY ((ARRAY['awaiting-approval'::character varying, 'started'::character varying])::text[])) = (jsonb_array_length(checks) > 0))))
);

CREATE TABLE scope_request_claims (
    request_id character varying NOT NULL,
    claimer_user_id character varying NOT NULL,
    claimed_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_claim_values CHECK (((claimed_at_unix >= 0) AND (updated_at_unix >= claimed_at_unix)))
);

CREATE TABLE scope_request_discussion_read_states (
    discussion_id character varying NOT NULL,
    user_id character varying NOT NULL,
    read_through_position bigint CONSTRAINT scope_request_discussion_read_st_read_through_position_not_null NOT NULL,
    updated_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_discussion_read_values CHECK (((read_through_position >= 0) AND (updated_at_unix >= 0)))
);

CREATE TABLE scope_request_discussion_replies (
    id character varying NOT NULL,
    discussion_id character varying NOT NULL,
    "position" bigint NOT NULL,
    author_user_id character varying,
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
    author_user_id character varying,
    body_markdown text NOT NULL,
    status character varying NOT NULL,
    client_discussion_id character varying NOT NULL,
    created_at_unix bigint NOT NULL,
    resolved_at_unix bigint,
    resolved_by_user_id character varying,
    revision_id character varying,
    commit_oid character varying,
    path text,
    CONSTRAINT scope_request_discussion_values CHECK (((opened_position > 0) AND (last_activity_position >= opened_position) AND ((status)::text = ANY (ARRAY[('Open'::character varying)::text, ('Resolved'::character varying)::text])) AND (length(btrim(body_markdown)) > 0) AND (created_at_unix >= 0) AND ((resolved_at_unix IS NULL) OR (resolved_at_unix >= 0)) AND ((commit_oid IS NULL) OR (revision_id IS NOT NULL)) AND ((path IS NULL) OR (commit_oid IS NOT NULL))))
);

CREATE TABLE scope_request_events (
    id character varying NOT NULL,
    request_id character varying NOT NULL,
    actor_user_id character varying,
    kind character varying NOT NULL,
    "position" bigint NOT NULL,
    payload jsonb NOT NULL,
    created_at_unix bigint NOT NULL,
    CONSTRAINT scope_request_event_values CHECK ((("position" > 0) AND (created_at_unix >= 0)))
);

CREATE TABLE scope_request_invitees (
    request_id character varying NOT NULL,
    user_id character varying NOT NULL,
    invited_by_user_id character varying,
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
    reserved_derivative_bytes bigint CONSTRAINT scope_request_media_attachme_reserved_derivative_bytes_not_null NOT NULL,
    actual_derivative_bytes bigint,
    created_at_unix bigint NOT NULL,
    updated_at_unix bigint NOT NULL,
    upload_expires_at_unix bigint NOT NULL,
    unbound_expires_at_unix bigint,
    budget_released_at_unix bigint,
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
    CONSTRAINT scope_request_media_attachments_video_width_check CHECK (((video_width IS NULL) OR (video_width > 0))),
    CONSTRAINT scope_request_media_budget_release_time CHECK (((budget_released_at_unix IS NULL) OR (budget_released_at_unix >= 0)))
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
    plaintext_size_bytes bigint CONSTRAINT scope_request_media_manifest_chun_plaintext_size_bytes_not_null NOT NULL,
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
    attachment_id text CONSTRAINT scope_request_media_orphan_cleanup_lease_attachment_id_not_null NOT NULL,
    lease_token text NOT NULL,
    lease_generation bigint CONSTRAINT scope_request_media_orphan_cleanup_le_lease_generation_not_null NOT NULL,
    lease_expires_at_unix bigint CONSTRAINT scope_request_media_orphan_clean_lease_expires_at_unix_not_null NOT NULL,
    attempt integer NOT NULL,
    updated_at_unix bigint CONSTRAINT scope_request_media_orphan_cleanup_lea_updated_at_unix_not_null NOT NULL,
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
    lease_generation bigint CONSTRAINT scope_request_media_processing_object_lease_generation_not_null NOT NULL,
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

CREATE TABLE scope_request_ref_cleanup_jobs (
    id text NOT NULL,
    repo_id text NOT NULL,
    incarnation_id text NOT NULL,
    request_id text NOT NULL,
    request_name text NOT NULL,
    head_oid text NOT NULL,
    created_at_unix bigint NOT NULL,
    next_run_at_unix bigint NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    last_error text,
    CONSTRAINT scope_request_ref_cleanup_jobs_attempts_check CHECK ((attempts >= 0)),
    CONSTRAINT scope_request_ref_cleanup_jobs_created_at_unix_check CHECK ((created_at_unix >= 0)),
    CONSTRAINT scope_request_ref_cleanup_jobs_next_run_at_unix_check CHECK ((next_run_at_unix >= 0))
);

CREATE TABLE scope_request_revisions (
    id character varying NOT NULL,
    request_id character varying NOT NULL,
    "position" bigint NOT NULL,
    actor_user_id character varying,
    old_head_oid character varying NOT NULL,
    new_head_oid character varying NOT NULL,
    git_snapshot jsonb NOT NULL,
    created_at_unix bigint NOT NULL,
    base_main_oid character varying NOT NULL,
    rewrote_history boolean NOT NULL,
    CONSTRAINT scope_request_revision_base CHECK ((length((base_main_oid)::text) > 0)),
    CONSTRAINT scope_request_revision_values CHECK ((("position" > 0) AND (created_at_unix >= 0) AND (length((old_head_oid)::text) > 0) AND (length((new_head_oid)::text) > 0)))
);

CREATE TABLE scope_requests (
    id character varying NOT NULL,
    repo_id character varying NOT NULL,
    name character varying NOT NULL,
    author_user_id character varying,
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
    CONSTRAINT scope_request_identity_values CHECK ((((name)::text ~ '^[a-z0-9][a-z0-9-]{0,47}$'::text) AND ((name)::text <> ALL ((ARRAY['main'::character varying, 'head'::character varying, 'scope'::character varying])::text[])) AND (length(btrim((audience)::text)) > 0) AND ((author_role)::text = ANY ((ARRAY['Public'::character varying, 'Member'::character varying, 'Owner'::character varying])::text[])))),
    CONSTRAINT scope_request_merge_coherence CHECK ((((merged_at_unix IS NULL) AND (merged_by_user_id IS NULL) AND (merged_head_oid IS NULL) AND (merged_main_oid IS NULL)) OR ((submitted_at_unix IS NOT NULL) AND (merged_at_unix IS NOT NULL) AND (merged_head_oid IS NOT NULL) AND (length((merged_head_oid)::text) > 0) AND (merged_main_oid IS NOT NULL) AND (length((merged_main_oid)::text) > 0)))),
    CONSTRAINT scope_request_nonnegative_values CHECK (((activity_version >= 0) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix) AND ((submitted_at_unix IS NULL) OR ((submitted_at_unix >= created_at_unix) AND (submitted_at_unix <= updated_at_unix))) AND ((closed_at_unix IS NULL) OR ((closed_at_unix >= created_at_unix) AND (closed_at_unix <= updated_at_unix))) AND ((merged_at_unix IS NULL) OR ((merged_at_unix >= created_at_unix) AND (merged_at_unix <= updated_at_unix))))),
    CONSTRAINT scope_request_submission_coherence CHECK ((((closed_at_unix IS NULL) OR (merged_at_unix IS NULL)) AND ((closed_at_unix IS NOT NULL) OR (closed_by_user_id IS NULL)) AND (((submitted_at_unix IS NULL) AND (closed_at_unix IS NULL) AND (merged_at_unix IS NULL)) OR ((submitted_at_unix IS NOT NULL) AND ((closed_at_unix IS NULL) OR (closed_at_unix >= submitted_at_unix)) AND ((merged_at_unix IS NULL) OR (merged_at_unix >= submitted_at_unix))))))
);

CREATE TABLE scope_run_attempt_cache_setups (
    attempt_id text NOT NULL,
    authorization_ms bigint NOT NULL,
    wall_ms bigint NOT NULL,
    CONSTRAINT scope_run_attempt_cache_setups_timings CHECK (((authorization_ms >= 0) AND (authorization_ms <= 86400000) AND ((wall_ms >= 0) AND (wall_ms <= 86400000)) AND (authorization_ms <= wall_ms)))
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
    CONSTRAINT scope_run_attempt_caches_preparation_timings CHECK (((key_ms >= 0) AND (key_ms <= 86400000) AND ((metadata_ms >= 0) AND (metadata_ms <= 86400000)) AND ((size_bytes >= 0) AND (size_bytes <= 1073741824)) AND ((download_verify_ms >= 0) AND (download_verify_ms <= 86400000)) AND ((sync_ms >= 0) AND (sync_ms <= 86400000)) AND ((extraction_ms >= 0) AND (extraction_ms <= 86400000)) AND (prepare_ms = ((((key_ms + metadata_ms) + download_verify_ms) + sync_ms) + extraction_ms)))),
    CONSTRAINT scope_run_attempt_caches_prepare_duration CHECK (((prepare_ms >= 0) AND (prepare_ms <= 86400000)))
);

CREATE TABLE scope_run_attempt_steps (
    attempt_id character varying NOT NULL,
    step_index integer NOT NULL,
    state character varying NOT NULL,
    started_at_unix bigint,
    completed_at_unix bigint,
    exit_code integer,
    CONSTRAINT scope_run_attempt_steps_values CHECK (((step_index >= 0) AND ((state)::text = ANY (ARRAY[('pending'::character varying)::text, ('running'::character varying)::text, ('succeeded'::character varying)::text, ('failed'::character varying)::text, ('canceled'::character varying)::text, ('lost'::character varying)::text, ('skipped'::character varying)::text])) AND (((state)::text = ANY (ARRAY[('succeeded'::character varying)::text, ('failed'::character varying)::text, ('canceled'::character varying)::text, ('lost'::character varying)::text, ('skipped'::character varying)::text])) = (completed_at_unix IS NOT NULL)) AND ((started_at_unix IS NULL) OR (completed_at_unix IS NULL) OR (completed_at_unix >= started_at_unix)) AND (((state)::text <> 'pending'::text) OR ((started_at_unix IS NULL) AND (completed_at_unix IS NULL) AND (exit_code IS NULL))) AND (((state)::text <> 'running'::text) OR ((started_at_unix IS NOT NULL) AND (completed_at_unix IS NULL) AND (exit_code IS NULL))) AND (((state)::text <> 'succeeded'::text) OR ((started_at_unix IS NOT NULL) AND (exit_code = 0))) AND (((state)::text <> 'failed'::text) OR ((started_at_unix IS NOT NULL) AND (exit_code IS NOT NULL) AND (exit_code <> 0))) AND (((state)::text = ANY (ARRAY[('failed'::character varying)::text, ('succeeded'::character varying)::text])) OR (exit_code IS NULL)) AND (((state)::text <> 'skipped'::text) OR (started_at_unix IS NULL)) AND (((state)::text <> ALL (ARRAY[('canceled'::character varying)::text, ('lost'::character varying)::text])) OR (started_at_unix IS NOT NULL))))
);

CREATE TABLE scope_run_attempts (
    id character varying NOT NULL,
    run_id character varying NOT NULL,
    number integer NOT NULL,
    token_hash character varying NOT NULL,
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
    CONSTRAINT scope_run_attempts_values CHECK (((number > 0) AND ((external_run_id IS NULL) OR (char_length(external_run_id) > 0)) AND ((runner_stop_claimed_at_unix IS NULL) OR (runner_stop_claimed_at_unix >= created_at_unix)) AND ((runner_stop_completed_at_unix IS NULL) OR ((runner_stop_claimed_at_unix IS NOT NULL) AND (runner_stop_completed_at_unix >= runner_stop_claimed_at_unix))) AND ((char_length(runtime_version) >= 1) AND (char_length(runtime_version) <= 128)) AND (char_length((token_hash)::text) = 64) AND ((token_hash)::text ~ '^[0-9A-Fa-f]+$'::text) AND ((state)::text = ANY (ARRAY[('dispatching'::character varying)::text, ('running'::character varying)::text, ('succeeded'::character varying)::text, ('failed'::character varying)::text, ('canceled'::character varying)::text, ('lost'::character varying)::text])) AND (created_at_unix >= 0) AND (last_heartbeat_at_unix >= created_at_unix) AND (last_heartbeat_at_unix < lease_expires_at_unix) AND ((started_at_unix IS NULL) OR ((started_at_unix >= created_at_unix) AND (started_at_unix < lease_expires_at_unix))) AND ((completed_at_unix IS NULL) OR (completed_at_unix >= last_heartbeat_at_unix)) AND ((started_at_unix IS NULL) OR (completed_at_unix IS NULL) OR (completed_at_unix >= started_at_unix)) AND (log_bytes >= 0) AND (log_bytes <= 10485760) AND (((state)::text = ANY (ARRAY[('succeeded'::character varying)::text, ('failed'::character varying)::text, ('canceled'::character varying)::text, ('lost'::character varying)::text])) = (completed_at_unix IS NOT NULL)) AND (((state)::text <> 'succeeded'::text) OR ((started_at_unix IS NOT NULL) AND (terminal_reason IS NULL))) AND (((state)::text <> ALL (ARRAY[('failed'::character varying)::text, ('canceled'::character varying)::text, ('lost'::character varying)::text])) OR (terminal_reason IS NOT NULL)) AND (((state)::text = ANY (ARRAY[('failed'::character varying)::text, ('canceled'::character varying)::text, ('lost'::character varying)::text])) OR (terminal_reason IS NULL))))
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
    CONSTRAINT scope_runs_values CHECK (((char_length((workflow_revision_digest)::text) = 64) AND ((workflow_revision_digest)::text ~ '^[0-9A-Fa-f]+$'::text) AND ((((source ->> 'kind'::text) = 'ephemeral-git-bundle'::text) AND (char_length((source #>> '{object,sha256}'::text[])) = 64) AND ((source #>> '{object,sha256}'::text[]) ~ '^[0-9A-Fa-f]+$'::text) AND (char_length((source #>> '{object,git_oid}'::text[])) = 40) AND ((source #>> '{object,git_oid}'::text[]) ~ '^[0-9A-Fa-f]+$'::text)) OR (((source ->> 'kind'::text) = 'request-git-snapshot'::text) AND (char_length((source #>> '{object,sha256}'::text[])) = 64) AND ((source #>> '{object,sha256}'::text[]) ~ '^[0-9A-Fa-f]+$'::text) AND (char_length((source #>> '{object,git_oid}'::text[])) = 40) AND ((source #>> '{object,git_oid}'::text[]) ~ '^[0-9A-Fa-f]+$'::text) AND (char_length((source ->> 'base_oid'::text)) = 40) AND ((source ->> 'base_oid'::text) ~ '^[0-9A-Fa-f]+$'::text) AND ((trigger)::text = 'request'::text)) OR (((source ->> 'kind'::text) = 'accepted-git-head'::text) AND (length(btrim((source ->> 'repository_id'::text))) > 0) AND (((source #>> '{head,push_sequence}'::text[]))::numeric > (0)::numeric) AND (((source #>> '{head,change_version}'::text[]))::numeric > (0)::numeric) AND (char_length((source #>> '{head,head_oid}'::text[])) = 40) AND ((source #>> '{head,head_oid}'::text[]) ~ '^[0-9A-Fa-f]+$'::text) AND (char_length((source #>> '{head,frontier}'::text[])) = 64) AND ((source #>> '{head,frontier}'::text[]) ~ '^[0-9A-Fa-f]+$'::text) AND (jsonb_typeof((source -> 'pack_spans'::text)) = 'array'::text) AND (jsonb_array_length((source -> 'pack_spans'::text)) > 0) AND (((((source -> 'pack_spans'::text) -> (jsonb_array_length((source -> 'pack_spans'::text)) - 1)) ->> 'last_sequence'::text))::numeric = ((source #>> '{head,push_sequence}'::text[]))::numeric) AND ((((source -> 'pack_spans'::text) -> (jsonb_array_length((source -> 'pack_spans'::text)) - 1)) ->> 'head_oid'::text) = (source #>> '{head,head_oid}'::text[])))) AND ((trigger)::text = ANY ((ARRAY['manual'::character varying, 'push-main'::character varying, 'request'::character varying])::text[])) AND ((state)::text = ANY ((ARRAY['queued'::character varying, 'dispatching'::character varying, 'running'::character varying, 'succeeded'::character varying, 'failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix) AND (((state)::text = ANY ((ARRAY['succeeded'::character varying, 'failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) = (completed_at_unix IS NOT NULL)) AND ((completed_at_unix IS NULL) OR (completed_at_unix = updated_at_unix)) AND (((state)::text <> 'canceled'::text) OR cancellation_requested)))
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
    capacity_retry_first_rejected_at_unix bigint,
    capacity_retry_rejections integer DEFAULT 0 NOT NULL,
    capacity_retry_next_attempt_at_unix bigint,
    CONSTRAINT scope_run_jobs_capacity_retry_values CHECK ((((capacity_retry_first_rejected_at_unix IS NULL) = (capacity_retry_rejections = 0)) AND ((capacity_retry_rejections >= 0) AND (capacity_retry_rejections <= 4)) AND ((capacity_retry_first_rejected_at_unix IS NULL) OR (capacity_retry_first_rejected_at_unix >= created_at_unix)) AND ((capacity_retry_next_attempt_at_unix IS NULL) OR (((state)::text = 'queued'::text) AND ((capacity_retry_rejections >= 1) AND (capacity_retry_rejections <= 3)) AND (capacity_retry_next_attempt_at_unix > capacity_retry_first_rejected_at_unix) AND (capacity_retry_next_attempt_at_unix < (capacity_retry_first_rejected_at_unix + 120)))))),
    CONSTRAINT scope_run_jobs_values CHECK (((char_length((job_key)::text) >= 1) AND (char_length((job_key)::text) <= 64) AND ((job_key)::text ~ '^[a-z0-9]+(-[a-z0-9]+)*$'::text) AND (pinned_container_image ~ '^[^@[:space:]]+@sha256:[0-9A-Fa-f]{64}$'::text) AND ((state)::text = ANY (ARRAY[('blocked'::character varying)::text, ('queued'::character varying)::text, ('dispatching'::character varying)::text, ('running'::character varying)::text, ('succeeded'::character varying)::text, ('failed'::character varying)::text, ('skipped'::character varying)::text, ('canceled'::character varying)::text, ('lost'::character varying)::text])) AND (last_attempt_number >= 0) AND (created_at_unix >= 0) AND (updated_at_unix >= created_at_unix) AND (((state)::text = ANY (ARRAY[('dispatching'::character varying)::text, ('running'::character varying)::text])) = (current_attempt_id IS NOT NULL)) AND (((state)::text = ANY (ARRAY[('succeeded'::character varying)::text, ('failed'::character varying)::text, ('skipped'::character varying)::text, ('canceled'::character varying)::text, ('lost'::character varying)::text])) = (completed_at_unix IS NOT NULL)) AND ((completed_at_unix IS NULL) OR (completed_at_unix = updated_at_unix))))
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
    views_before jsonb,
    views_after jsonb,
    CONSTRAINT scope_visibility_change_set_values CHECK (((ordinal >= 0) AND (char_length((id)::text) > 0) AND (char_length((author_id)::text) > 0))),
    CONSTRAINT scope_visibility_change_set_views CHECK ((((views_before IS NULL) = (views_after IS NULL)) AND ((views_before IS NULL) OR ((jsonb_typeof(views_before) = 'array'::text) AND (jsonb_typeof(views_after) = 'array'::text)))))
);

CREATE TABLE scope_visibility_changes (
    repo_id character varying NOT NULL,
    change_set_id character varying NOT NULL,
    ordinal bigint NOT NULL,
    path character varying NOT NULL,
    old_visibility character varying NOT NULL,
    new_visibility character varying NOT NULL,
    current_content jsonb,
    CONSTRAINT scope_visibility_change_values CHECK (((ordinal >= 0) AND (length((path)::text) > 0) AND (length(btrim((old_visibility)::text)) > 0) AND (length(btrim((new_visibility)::text)) > 0) AND ((old_visibility)::text <> (new_visibility)::text)))
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

ALTER TABLE ONLY scope_clerk_user_deletions
    ADD CONSTRAINT scope_clerk_user_deletions_pkey PRIMARY KEY (clerk_user_id);

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

ALTER TABLE ONLY scope_dependency_analyses
    ADD CONSTRAINT scope_dependency_analyses_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_dependency_analysis_jobs
    ADD CONSTRAINT scope_dependency_analysis_jobs_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_dependency_reports
    ADD CONSTRAINT scope_dependency_reports_pkey PRIMARY KEY (repo_id);

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

ALTER TABLE ONLY scope_github_check_refreshes
    ADD CONSTRAINT scope_github_check_refreshes_pkey PRIMARY KEY (repo_id, github_repository_id, commit_oid);

ALTER TABLE ONLY scope_github_check_runs
    ADD CONSTRAINT scope_github_check_runs_pkey PRIMARY KEY (github_check_run_id);

ALTER TABLE ONLY scope_github_connections
    ADD CONSTRAINT scope_github_connections_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_github_pushes
    ADD CONSTRAINT scope_github_pushes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_github_required_checks
    ADD CONSTRAINT scope_github_required_checks_pkey PRIMARY KEY (repo_id, name);

ALTER TABLE ONLY scope_github_run_import_counts
    ADD CONSTRAINT scope_github_run_import_counts_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_github_run_imports
    ADD CONSTRAINT scope_github_run_imports_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_github_setup_checks
    ADD CONSTRAINT scope_github_setup_checks_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_github_workflow_job_log_reads
    ADD CONSTRAINT scope_github_workflow_job_log_reads_pkey PRIMARY KEY (github_job_id);

ALTER TABLE ONLY scope_github_workflow_job_logs
    ADD CONSTRAINT scope_github_workflow_job_logs_pkey PRIMARY KEY (github_job_id);

ALTER TABLE ONLY scope_github_workflow_job_reads
    ADD CONSTRAINT scope_github_workflow_job_reads_pkey PRIMARY KEY (repo_id, github_repository_id, github_run_id);

ALTER TABLE ONLY scope_github_workflow_jobs
    ADD CONSTRAINT scope_github_workflow_jobs_pkey PRIMARY KEY (github_job_id);

ALTER TABLE ONLY scope_github_workflow_run_reads
    ADD CONSTRAINT scope_github_workflow_run_reads_pkey PRIMARY KEY (repo_id, github_repository_id, github_run_id);

ALTER TABLE ONLY scope_github_workflow_runs
    ADD CONSTRAINT scope_github_workflow_runs_pkey PRIMARY KEY (github_run_id);

ALTER TABLE ONLY scope_live_files
    ADD CONSTRAINT scope_live_files_pkey PRIMARY KEY (repo_id, path);

ALTER TABLE ONLY scope_logical_commits
    ADD CONSTRAINT scope_logical_commits_pkey PRIMARY KEY (repo_id, id);

ALTER TABLE ONLY scope_logical_commits
    ADD CONSTRAINT scope_logical_commits_repo_id_ordinal_key UNIQUE (repo_id, ordinal);

ALTER TABLE ONLY scope_metadata_locks
    ADD CONSTRAINT scope_metadata_locks_pkey PRIMARY KEY (key);

ALTER TABLE ONLY scope_native_runs_accounts
    ADD CONSTRAINT scope_native_runs_accounts_pkey PRIMARY KEY (user_id);

ALTER TABLE ONLY scope_object_references
    ADD CONSTRAINT scope_object_references_pkey PRIMARY KEY (object_key, ref_kind, ref_id);

ALTER TABLE ONLY scope_orphan_object_jobs
    ADD CONSTRAINT scope_orphan_object_jobs_pkey PRIMARY KEY (object_key);

ALTER TABLE ONLY scope_outbox_jobs
    ADD CONSTRAINT scope_outbox_jobs_idempotency_key_key UNIQUE (idempotency_key);

ALTER TABLE ONLY scope_outbox_jobs
    ADD CONSTRAINT scope_outbox_jobs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_projection_files
    ADD CONSTRAINT scope_projection_files_pkey PRIMARY KEY (repo_id, path_key);

ALTER TABLE ONLY scope_projection_read_models
    ADD CONSTRAINT scope_projection_read_models_pkey PRIMARY KEY (repo_id, audience);

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

ALTER TABLE ONLY scope_repository_history_payloads
    ADD CONSTRAINT scope_repository_history_payloads_pkey PRIMARY KEY (repo_id, payload_hash);

ALTER TABLE ONLY scope_repository_invite_emails
    ADD CONSTRAINT scope_repository_invite_emails_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_repository_invite_links
    ADD CONSTRAINT scope_repository_invite_links_pkey PRIMARY KEY (token_hash);

ALTER TABLE ONLY scope_repository_invites
    ADD CONSTRAINT scope_repository_invites_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_repository_landing_files
    ADD CONSTRAINT scope_repository_landing_files_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_repository_workflow_catalogs
    ADD CONSTRAINT scope_repository_workflow_catalogs_pkey PRIMARY KEY (repo_id);

ALTER TABLE ONLY scope_repository_workflow_files
    ADD CONSTRAINT scope_repository_workflow_files_pkey PRIMARY KEY (repo_id, path);

ALTER TABLE ONLY scope_request_attention_states
    ADD CONSTRAINT scope_request_attention_states_pkey PRIMARY KEY (request_id, user_id);

ALTER TABLE ONLY scope_request_auto_merge_intents
    ADD CONSTRAINT scope_request_auto_merge_intents_pkey PRIMARY KEY (id);

ALTER TABLE ONLY scope_request_check_evaluations
    ADD CONSTRAINT scope_request_check_evaluations_pkey PRIMARY KEY (request_id, head_oid);

ALTER TABLE ONLY scope_request_claims
    ADD CONSTRAINT scope_request_claims_pkey PRIMARY KEY (request_id);

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

ALTER TABLE ONLY scope_request_ref_cleanup_jobs
    ADD CONSTRAINT scope_request_ref_cleanup_jobs_pkey PRIMARY KEY (id);

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

CREATE INDEX idx_scope_cache_uploads_cleanup_lease ON scope_cache_uploads USING btree (cleanup_lease_expires_at_unix, upload_id) WHERE (state = 'deleting'::text);

CREATE INDEX idx_scope_cache_uploads_expiry ON scope_cache_uploads USING btree (expires_at_unix, upload_id);

CREATE INDEX idx_scope_clerk_user_deletions_due ON scope_clerk_user_deletions USING btree (next_attempt_at_unix, clerk_user_id);

CREATE INDEX idx_scope_cli_exchange_grants_user ON scope_cli_exchange_grants USING btree (user_id);

CREATE INDEX idx_scope_cli_sessions_user ON scope_cli_sessions USING btree (user_id);

CREATE INDEX idx_scope_file_changes_path ON scope_file_changes USING btree (repo_id, path);

CREATE INDEX idx_scope_git_segment_references_owner ON scope_git_segment_references USING btree (ref_kind, ref_id, segment_id);

CREATE INDEX idx_scope_git_segment_uploads_recovery ON scope_git_segment_uploads USING btree (state, updated_at_unix, segment_id) WHERE (state = ANY (ARRAY['uploading'::text, 'ready'::text, 'deleting'::text]));

CREATE INDEX idx_scope_github_check_runs_commit ON scope_github_check_runs USING btree (repo_id, github_repository_id, commit_oid, name, github_check_run_id DESC);

CREATE UNIQUE INDEX idx_scope_github_connections_connected_repository ON scope_github_connections USING btree (github_repository_id) WHERE ((status)::text = 'Connected'::text);

CREATE INDEX idx_scope_github_connections_installation ON scope_github_connections USING btree (installation_id) WHERE ((status)::text = 'Connected'::text);

CREATE INDEX idx_scope_github_pushes_due ON scope_github_pushes USING btree (next_attempt_at_unix) WHERE ((state)::text = ANY ((ARRAY['queued'::character varying, 'running'::character varying])::text[]));

CREATE UNIQUE INDEX idx_scope_github_pushes_ref ON scope_github_pushes USING btree (repo_id, ref, sequence);

CREATE INDEX idx_scope_github_pushes_request ON scope_github_pushes USING btree (request_id, sequence DESC);

CREATE INDEX idx_scope_github_run_imports_due ON scope_github_run_imports USING btree (next_attempt_at_unix) WHERE ((state)::text = ANY ((ARRAY['queued'::character varying, 'running'::character varying])::text[]));

CREATE INDEX idx_scope_github_setup_checks_running ON scope_github_setup_checks USING btree (started_at_unix) WHERE ((state)::text = ANY ((ARRAY['pushing'::character varying, 'waiting'::character varying])::text[]));

CREATE INDEX idx_scope_github_workflow_job_log_reads_due ON scope_github_workflow_job_log_reads USING btree (next_attempt_at_unix);

CREATE INDEX idx_scope_github_workflow_job_reads_due ON scope_github_workflow_job_reads USING btree (next_attempt_at_unix);

CREATE INDEX idx_scope_github_workflow_jobs_run ON scope_github_workflow_jobs USING btree (repo_id, github_repository_id, github_run_id, run_attempt);

CREATE INDEX idx_scope_github_workflow_run_reads_due ON scope_github_workflow_run_reads USING btree (next_attempt_at_unix);

CREATE INDEX idx_scope_github_workflow_runs_branch ON scope_github_workflow_runs USING btree (repo_id, github_repository_id, head_branch, head_oid);

CREATE INDEX idx_scope_github_workflow_runs_recent ON scope_github_workflow_runs USING btree (repo_id, github_repository_id, COALESCE(run_started_at_unix, github_updated_at_unix) DESC, github_run_id DESC);

CREATE INDEX idx_scope_github_workflow_runs_workflow_recent ON scope_github_workflow_runs USING btree (repo_id, github_repository_id, workflow_name, COALESCE(run_started_at_unix, github_updated_at_unix) DESC, github_run_id DESC);

CREATE INDEX idx_scope_orphan_object_jobs_pending ON scope_orphan_object_jobs USING btree (completed_at_unix, next_run_at_unix);

CREATE INDEX idx_scope_outbox_jobs_ready ON scope_outbox_jobs USING btree (state, next_run_at_unix, created_at_unix);

CREATE INDEX idx_scope_outbox_jobs_repo ON scope_outbox_jobs USING btree (repo_id, repo_version);

CREATE INDEX idx_scope_repo_storage_cleanup_jobs_pending ON scope_repo_storage_cleanup_jobs USING btree (completed_at_unix, next_run_at_unix);

CREATE UNIQUE INDEX idx_scope_repositories_owner_name ON scope_repositories USING btree (owner_handle, name);

CREATE INDEX idx_scope_repository_invite_emails_due ON scope_repository_invite_emails USING btree (next_attempt_at_unix, created_at_unix, id) WHERE ((state)::text = 'Queued'::text);

CREATE INDEX idx_scope_repository_invite_emails_invite ON scope_repository_invite_emails USING btree (invite_id, created_at_unix);

CREATE INDEX idx_scope_repository_invite_emails_requester ON scope_repository_invite_emails USING btree (requested_by_user_id, created_at_unix);

CREATE INDEX idx_scope_repository_invite_links_invite ON scope_repository_invite_links USING btree (invite_id);

CREATE INDEX idx_scope_repository_invites_repo_email ON scope_repository_invites USING btree (repo_id, invited_email_normalized);

CREATE INDEX idx_scope_repository_members_user ON scope_repository_members USING btree (user_id);

CREATE INDEX idx_scope_request_attention_queue ON scope_request_attention_states USING btree (user_id, state, snoozed_until_unix, request_id);

CREATE INDEX idx_scope_request_auto_merge_due ON scope_request_auto_merge_intents USING btree (next_attempt_at_unix, created_at_unix, id) WHERE ((status)::text = 'Active'::text);

CREATE INDEX idx_scope_request_auto_merge_repo_actor ON scope_request_auto_merge_intents USING btree (repo_id, actor_user_id) WHERE ((status)::text = 'Active'::text);

CREATE INDEX idx_scope_request_check_evaluations_checks ON scope_request_check_evaluations USING gin (checks jsonb_path_ops);

CREATE INDEX idx_scope_request_check_evaluations_tested_oid ON scope_request_check_evaluations USING btree (tested_oid);

CREATE INDEX idx_scope_request_claims_claimer ON scope_request_claims USING btree (claimer_user_id, request_id);

CREATE INDEX idx_scope_request_discussion_replies_chronological ON scope_request_discussion_replies USING btree (discussion_id, "position" DESC);

CREATE INDEX idx_scope_request_discussions_newest ON scope_request_discussions USING btree (request_id, opened_position DESC, id);

CREATE UNIQUE INDEX idx_scope_request_events_request_position ON scope_request_events USING btree (request_id, "position");

CREATE INDEX idx_scope_request_invitees_user ON scope_request_invitees USING btree (user_id, request_id);

CREATE INDEX idx_scope_request_ratings_subject ON scope_request_ratings USING btree (subject_user_id, created_at_unix, id);

CREATE INDEX idx_scope_request_ref_cleanup_ready ON scope_request_ref_cleanup_jobs USING btree (next_run_at_unix, id);

CREATE INDEX idx_scope_request_revisions_request_position ON scope_request_revisions USING btree (request_id, "position" DESC, id);

CREATE INDEX idx_scope_requests_author ON scope_requests USING btree (author_user_id);

CREATE INDEX idx_scope_requests_closed_queue ON scope_requests USING btree (repo_id, COALESCE(closed_at_unix, merged_at_unix) DESC, id) WHERE ((closed_at_unix IS NOT NULL) OR (merged_at_unix IS NOT NULL));

CREATE INDEX idx_scope_requests_draft_queue ON scope_requests USING btree (repo_id, updated_at_unix DESC, id) WHERE ((submitted_at_unix IS NULL) AND (closed_at_unix IS NULL) AND (merged_at_unix IS NULL));

CREATE INDEX idx_scope_requests_open_queue ON scope_requests USING btree (repo_id, submitted_at_unix, id) WHERE ((submitted_at_unix IS NOT NULL) AND (closed_at_unix IS NULL) AND (merged_at_unix IS NULL));

CREATE INDEX idx_scope_requests_public_closed_queue ON scope_requests USING btree (repo_id, COALESCE(closed_at_unix, merged_at_unix) DESC, id) WHERE (((audience)::text = 'public'::text) AND ((closed_at_unix IS NOT NULL) OR (merged_at_unix IS NOT NULL)));

CREATE INDEX idx_scope_requests_public_open_queue ON scope_requests USING btree (repo_id, submitted_at_unix, id) WHERE (((audience)::text = 'public'::text) AND (submitted_at_unix IS NOT NULL) AND (closed_at_unix IS NULL) AND (merged_at_unix IS NULL));

CREATE INDEX idx_scope_requests_public_search ON scope_requests USING gin (title public.gin_trgm_ops, description_markdown public.gin_trgm_ops) WHERE ((audience)::text = 'public'::text);

CREATE INDEX idx_scope_requests_repo_audience_id ON scope_requests USING btree (repo_id, audience, id);

CREATE INDEX idx_scope_requests_repo_id ON scope_requests USING btree (repo_id, id);

CREATE UNIQUE INDEX idx_scope_run_attempts_active ON scope_run_attempts USING btree (run_id, job_key) WHERE ((state)::text = ANY ((ARRAY['dispatching'::character varying, 'running'::character varying])::text[]));

CREATE INDEX idx_scope_run_attempts_expiring ON scope_run_attempts USING btree (lease_expires_at_unix, id) WHERE ((state)::text = ANY ((ARRAY['dispatching'::character varying, 'running'::character varying])::text[]));

CREATE UNIQUE INDEX idx_scope_run_attempts_external_run ON scope_run_attempts USING btree (external_run_id) WHERE (external_run_id IS NOT NULL);

CREATE INDEX idx_scope_run_attempts_state ON scope_run_attempts USING btree (state, created_at_unix);

CREATE INDEX idx_scope_run_jobs_capacity_retry_due ON scope_run_jobs USING btree (capacity_retry_next_attempt_at_unix) WHERE (((state)::text = 'queued'::text) AND (capacity_retry_next_attempt_at_unix IS NOT NULL));

CREATE INDEX idx_scope_run_jobs_dispatch ON scope_run_jobs USING btree (created_at_unix, run_id, job_key) WHERE ((state)::text = 'queued'::text);

CREATE INDEX idx_scope_run_logs_run_position ON scope_run_logs USING btree (run_id, "position");

CREATE INDEX idx_scope_run_logs_step_position ON scope_run_logs USING btree (attempt_id, step_index, "position");

CREATE INDEX idx_scope_runs_history ON scope_runs USING btree (repo_id, creation_sequence DESC);

CREATE INDEX idx_scope_runs_queue ON scope_runs USING btree (created_at_unix, id) WHERE ((state)::text = 'queued'::text);

CREATE INDEX idx_scope_runs_workflow_history ON scope_runs USING btree (repo_id, workflow_path, creation_sequence DESC);

CREATE UNIQUE INDEX idx_scope_users_email ON scope_users USING btree (email);

CREATE INDEX idx_scope_visibility_changes_path ON scope_visibility_changes USING btree (repo_id, path);

CREATE INDEX scope_dependency_analysis_jobs_due ON scope_dependency_analysis_jobs USING btree (next_run_at_unix, updated_at_unix, repo_id);

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

CREATE UNIQUE INDEX uq_scope_request_auto_merge_active ON scope_request_auto_merge_intents USING btree (request_id) WHERE ((status)::text = 'Active'::text);

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

ALTER TABLE ONLY scope_push_trigger_evaluations
    ADD CONSTRAINT fk_scope_push_trigger_evaluations_repo FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repositories
    ADD CONSTRAINT fk_scope_repositories_owner FOREIGN KEY (owner_user_id) REFERENCES scope_users(id) ON DELETE RESTRICT;

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

ALTER TABLE ONLY scope_request_auto_merge_intents
    ADD CONSTRAINT fk_scope_request_auto_merge_revision FOREIGN KEY (request_id, revision_id) REFERENCES scope_request_revisions(request_id, id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussion_read_states
    ADD CONSTRAINT fk_scope_request_discussion_read_states_discussion FOREIGN KEY (discussion_id) REFERENCES scope_request_discussions(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussion_read_states
    ADD CONSTRAINT fk_scope_request_discussion_read_states_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT fk_scope_request_discussion_replies_author FOREIGN KEY (author_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT fk_scope_request_discussion_replies_discussion FOREIGN KEY (discussion_id) REFERENCES scope_request_discussions(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussion_replies
    ADD CONSTRAINT fk_scope_request_discussion_replies_quoted_reply FOREIGN KEY (reply_to_reply_id) REFERENCES scope_request_discussion_replies(id) ON DELETE RESTRICT;

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_author FOREIGN KEY (author_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_resolver FOREIGN KEY (resolved_by_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_request_discussions
    ADD CONSTRAINT fk_scope_request_discussions_revision FOREIGN KEY (request_id, revision_id) REFERENCES scope_request_revisions(request_id, id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_events
    ADD CONSTRAINT fk_scope_request_events_actor FOREIGN KEY (actor_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_request_events
    ADD CONSTRAINT fk_scope_request_events_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT fk_scope_request_invitees_inviter FOREIGN KEY (invited_by_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT fk_scope_request_invitees_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_invitees
    ADD CONSTRAINT fk_scope_request_invitees_user FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_ratings
    ADD CONSTRAINT fk_scope_request_ratings_rater FOREIGN KEY (rater_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_ratings
    ADD CONSTRAINT fk_scope_request_ratings_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_ratings
    ADD CONSTRAINT fk_scope_request_ratings_subject FOREIGN KEY (subject_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_revisions
    ADD CONSTRAINT fk_scope_request_revisions_actor FOREIGN KEY (actor_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_request_revisions
    ADD CONSTRAINT fk_scope_request_revisions_request FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_author FOREIGN KEY (author_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_closer FOREIGN KEY (closed_by_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_requests
    ADD CONSTRAINT fk_scope_requests_merger FOREIGN KEY (merged_by_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

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
    ADD CONSTRAINT fk_scope_runs_requester FOREIGN KEY (requested_by_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_runs
    ADD CONSTRAINT fk_scope_runs_revision FOREIGN KEY (workflow_revision_digest) REFERENCES scope_workflow_revisions(digest) ON DELETE RESTRICT;

ALTER TABLE ONLY scope_cache_objects
    ADD CONSTRAINT scope_cache_objects_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_cache_orphan_uploads
    ADD CONSTRAINT scope_cache_orphan_uploads_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_cache_uploads
    ADD CONSTRAINT scope_cache_uploads_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_dependency_analyses
    ADD CONSTRAINT scope_dependency_analyses_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_dependency_analysis_jobs
    ADD CONSTRAINT scope_dependency_analysis_jobs_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_dependency_reports
    ADD CONSTRAINT scope_dependency_reports_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_git_compaction_jobs
    ADD CONSTRAINT scope_git_compaction_jobs_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_git_segment_references
    ADD CONSTRAINT scope_git_segment_references_segment_id_fkey FOREIGN KEY (segment_id) REFERENCES scope_git_segment_uploads(segment_id) ON DELETE RESTRICT;

ALTER TABLE ONLY scope_github_check_refreshes
    ADD CONSTRAINT scope_github_check_refreshes_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_check_runs
    ADD CONSTRAINT scope_github_check_runs_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_connections
    ADD CONSTRAINT scope_github_connections_connected_by_user_id_fkey FOREIGN KEY (connected_by_user_id) REFERENCES scope_users(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_github_connections
    ADD CONSTRAINT scope_github_connections_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_required_checks
    ADD CONSTRAINT scope_github_required_checks_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_run_import_counts
    ADD CONSTRAINT scope_github_run_import_counts_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_run_imports
    ADD CONSTRAINT scope_github_run_imports_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_setup_checks
    ADD CONSTRAINT scope_github_setup_checks_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_workflow_job_log_reads
    ADD CONSTRAINT scope_github_workflow_job_log_reads_github_job_id_fkey FOREIGN KEY (github_job_id) REFERENCES scope_github_workflow_jobs(github_job_id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_workflow_job_logs
    ADD CONSTRAINT scope_github_workflow_job_logs_github_job_id_fkey FOREIGN KEY (github_job_id) REFERENCES scope_github_workflow_jobs(github_job_id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_workflow_job_reads
    ADD CONSTRAINT scope_github_workflow_job_reads_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_workflow_jobs
    ADD CONSTRAINT scope_github_workflow_jobs_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_workflow_run_reads
    ADD CONSTRAINT scope_github_workflow_run_reads_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_github_workflow_runs
    ADD CONSTRAINT scope_github_workflow_runs_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_native_runs_accounts
    ADD CONSTRAINT scope_native_runs_accounts_user_id_fkey FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_projection_files
    ADD CONSTRAINT scope_projection_files_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_projection_read_models
    ADD CONSTRAINT scope_projection_read_models_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_history_entries
    ADD CONSTRAINT scope_repository_history_entries_repo_id_audience_fkey FOREIGN KEY (repo_id, audience) REFERENCES scope_projection_read_models(repo_id, audience) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_history_entries
    ADD CONSTRAINT scope_repository_history_entries_repo_id_payload_hash_fkey FOREIGN KEY (repo_id, payload_hash) REFERENCES scope_repository_history_payloads(repo_id, payload_hash);

ALTER TABLE ONLY scope_repository_history_payloads
    ADD CONSTRAINT scope_repository_history_payloads_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_invite_emails
    ADD CONSTRAINT scope_repository_invite_emails_invite_id_fkey FOREIGN KEY (invite_id) REFERENCES scope_repository_invites(id) ON DELETE SET NULL;

ALTER TABLE ONLY scope_repository_invite_emails
    ADD CONSTRAINT scope_repository_invite_emails_requested_by_user_id_fkey FOREIGN KEY (requested_by_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_invite_links
    ADD CONSTRAINT scope_repository_invite_links_invite_id_fkey FOREIGN KEY (invite_id) REFERENCES scope_repository_invites(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_landing_files
    ADD CONSTRAINT scope_repository_landing_files_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_workflow_catalogs
    ADD CONSTRAINT scope_repository_workflow_catalogs_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_repository_workflow_files
    ADD CONSTRAINT scope_repository_workflow_files_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repository_workflow_catalogs(repo_id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_attention_states
    ADD CONSTRAINT scope_request_attention_states_request_id_fkey FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_attention_states
    ADD CONSTRAINT scope_request_attention_states_user_id_fkey FOREIGN KEY (user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_auto_merge_intents
    ADD CONSTRAINT scope_request_auto_merge_intents_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_auto_merge_intents
    ADD CONSTRAINT scope_request_auto_merge_intents_repo_id_fkey FOREIGN KEY (repo_id) REFERENCES scope_repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_auto_merge_intents
    ADD CONSTRAINT scope_request_auto_merge_intents_repository_incarnation_id_fkey FOREIGN KEY (repository_incarnation_id) REFERENCES scope_repositories(incarnation_id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_auto_merge_intents
    ADD CONSTRAINT scope_request_auto_merge_intents_request_id_fkey FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_check_evaluations
    ADD CONSTRAINT scope_request_check_evaluations_request_id_fkey FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_claims
    ADD CONSTRAINT scope_request_claims_claimer_user_id_fkey FOREIGN KEY (claimer_user_id) REFERENCES scope_users(id) ON DELETE CASCADE;

ALTER TABLE ONLY scope_request_claims
    ADD CONSTRAINT scope_request_claims_request_id_fkey FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE;

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
