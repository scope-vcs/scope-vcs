use super::*;

const REPOSITORY_ID: &str = "cache-owner/cache-repo";
const OBJECT_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const IDENTITY_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[tokio::test]
async fn cache_service_schema_enforces_content_and_lifecycle_invariants() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    db.execute_unprepared(&format!(
        "
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('cache-user', 'cache-owner', 'cache@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy, incarnation_id
        ) VALUES (
            '{REPOSITORY_ID}', 'cache-owner', 'cache-repo', 'cache-user', 'Ready',
            1, '{{}}'::jsonb, '{{}}'::jsonb, 'repoi_cache_owner_repo'
        );
        INSERT INTO scope_cache_objects (
            repository_id, checksum_sha256, storage_backend, object_key,
            size_bytes, created_at_unix, last_accessed_at_unix
        ) VALUES (
            '{REPOSITORY_ID}', '{OBJECT_DIGEST}', 'railway-iad',
            'repos/{REPOSITORY_ID}/objects/sha256/{OBJECT_DIGEST}',
            1024, 10, 10
        );
        INSERT INTO scope_cache_references (
            repository_id, identity_digest, compatibility_group_digest,
            checksum_sha256, created_at_unix, expires_at_unix, last_accessed_at_unix
        ) VALUES (
            '{REPOSITORY_ID}', '{IDENTITY_DIGEST}', repeat('c', 64), '{OBJECT_DIGEST}',
            10, 604810, 10
        );
        INSERT INTO scope_cache_uploads (
            upload_id, repository_id, identity_digest, compatibility_group_digest,
            checksum_sha256, storage_backend, object_key, size_bytes,
            state, created_at_unix, expires_at_unix
        ) VALUES (
            'upload-1', '{REPOSITORY_ID}', repeat('d', 64), repeat('e', 64), repeat('f', 64),
            'railway-iad',
            'repos/{REPOSITORY_ID}/objects/sha256/' || repeat('f', 64),
            2048, 'active', 20, 1820
        );
        INSERT INTO scope_cache_deletion_queue (
            repository_id, checksum_sha256, not_before_unix, attempts, last_error
        ) VALUES ('{REPOSITORY_ID}', '{OBJECT_DIGEST}', 3610, 0, NULL);
        "
    ))
    .await
    .unwrap();

    for index in [
        "idx_scope_cache_objects_access",
        "idx_scope_cache_references_object",
        "idx_scope_cache_references_expiry",
        "idx_scope_cache_references_access",
        "idx_scope_cache_uploads_expiry",
        "idx_scope_cache_uploads_active_identity",
        "idx_scope_cache_orphan_uploads_due",
        "idx_scope_cache_deletion_queue_due",
    ] {
        assert!(relation_exists(db.as_ref(), index).await, "missing {index}");
    }

    assert!(
        db.execute_unprepared(&format!(
            "INSERT INTO scope_cache_objects (
                repository_id, checksum_sha256, storage_backend, object_key,
                size_bytes, created_at_unix, last_accessed_at_unix
            ) VALUES (
                '{REPOSITORY_ID}', repeat('A', 64), 'railway-iad',
                'repos/{REPOSITORY_ID}/objects/sha256/' || repeat('A', 64),
                1, 0, 0
            )"
        ))
        .await
        .is_err(),
        "object digests must be lowercase SHA-256"
    );
    assert!(
        db.execute_unprepared(&format!(
            "INSERT INTO scope_cache_uploads (
                upload_id, repository_id, identity_digest, compatibility_group_digest,
                checksum_sha256,
                storage_backend, object_key, size_bytes,
                state, created_at_unix, expires_at_unix
            ) VALUES (
                'oversized', '{REPOSITORY_ID}', repeat('e', 64), repeat('d', 64),
                repeat('f', 64),
                'railway-iad',
                'repos/{REPOSITORY_ID}/objects/sha256/' || repeat('f', 64),
                1073741825, 'active', 0, 1800
            )"
        ))
        .await
        .is_err(),
        "the launch object cap must be enforced"
    );
    assert!(
        db.execute_unprepared(&format!(
            "INSERT INTO scope_cache_references (
                repository_id, identity_digest, compatibility_group_digest,
                checksum_sha256, created_at_unix, expires_at_unix, last_accessed_at_unix
            ) VALUES (
                '{REPOSITORY_ID}', repeat('f', 64), repeat('d', 64), repeat('e', 64),
                10, 20, 10
            )"
        ))
        .await
        .is_err(),
        "references cannot name uncommitted objects"
    );

    db.execute_unprepared(&format!(
        "DELETE FROM scope_repositories WHERE id = '{REPOSITORY_ID}'"
    ))
    .await
    .unwrap();
    for table in [
        "scope_cache_objects",
        "scope_cache_references",
        "scope_cache_uploads",
        "scope_cache_orphan_uploads",
        "scope_cache_deletion_queue",
    ] {
        let count = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT count(*) AS count FROM {table}"),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "count")
            .unwrap();
        assert_eq!(count, 0, "repository deletion must clear {table}");
    }
}
