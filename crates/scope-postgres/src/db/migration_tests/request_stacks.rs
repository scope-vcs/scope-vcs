use super::*;

#[tokio::test]
async fn stack_migration_preserves_main_and_enforces_parent_pins_and_receipt_identity() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0086_private_request_stacks")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('stack-owner', 'stack-owner', 'stack-owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id
        ) VALUES (
            'stack-owner/repo', 'stack-owner', 'repo', 'stack-owner', 'Ready', 1, 1,
            '{}', '{}', 'repoi_stack_migration'
        );
        INSERT INTO scope_requests (
            id, repo_id, name, author_user_id, author_role, audience,
            base_kind, review_base_oid, base_version, main_ancestor_oid, head_oid,
            title, description_markdown, activity_version, created_at_unix, updated_at_unix
        ) SELECT name, 'stack-owner/repo', name, 'stack-owner', 'Owner', 'private',
                 'Main', repeat('a',40), 1, repeat('a',40), repeat('b',40), name, '', 1, 1, 1
          FROM (VALUES ('parent'), ('child'), ('other')) AS fixture(name);
        INSERT INTO scope_request_revisions (
            id, request_id, position, actor_user_id, old_head_oid, new_head_oid,
            base_kind, review_base_oid, main_ancestor_oid, rewrote_history, git_snapshot, created_at_unix
        ) SELECT name || '-revision', name, 1, 'stack-owner', repeat('a',40), repeat('b',40),
                 'Main', repeat('a',40), repeat('a',40), FALSE, '{}', 1
          FROM (VALUES ('parent'), ('child'), ('other')) AS fixture(name);
        UPDATE scope_requests SET submitted_at_unix = 1,
            git_snapshot = jsonb_build_object(
                'content_ref', jsonb_build_object('GitBundleSha256', repeat('f',64)),
                'sha256', repeat('f',64), 'git_oid', repeat('b',40),
                'git_file_mode', '100644', 'size_bytes', 1
            ) WHERE id = 'parent';
        UPDATE scope_request_revisions SET git_snapshot = jsonb_build_object(
            'content_ref', jsonb_build_object('GitBundleSha256', repeat('f',64)),
            'sha256', repeat('f',64), 'git_oid', repeat('b',40),
            'git_file_mode', '100644', 'size_bytes', 1
        );
    "#).await.unwrap();
    migrations::Migrator::up(db.as_ref(), None).await.unwrap();
    let main = db.query_one_raw(Statement::from_string(DatabaseBackend::Postgres,
        "SELECT stack_status, stack_depth, base_kind, review_base_oid FROM scope_requests WHERE id = 'parent'"
    )).await.unwrap().unwrap();
    assert_eq!(main.try_get::<String>("", "stack_status").unwrap(), "main");
    assert_eq!(main.try_get::<i32>("", "stack_depth").unwrap(), 1);
    assert_eq!(main.try_get::<String>("", "base_kind").unwrap(), "Main");
    assert_eq!(
        main.try_get::<String>("", "review_base_oid").unwrap(),
        "a".repeat(40)
    );
    let open_parent = crate::db::request_rows::request_by_id(db.as_ref(), "parent")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        open_parent.state(),
        scope_domain::requests::RequestState::Open
    );
    assert_eq!(
        open_parent.git_snapshot.as_ref().unwrap().git_oid,
        "b".repeat(40)
    );
    db.execute_unprepared(
        r#"
        UPDATE scope_requests SET base_kind = 'Request', parent_request_id = 'parent',
            parent_revision_id = 'parent-revision', parent_head_oid = repeat('b',40),
            review_base_oid = repeat('b',40), stack_status = 'waiting_for_parent', stack_depth = 2
            WHERE id = 'child';
        UPDATE scope_request_revisions SET base_kind = 'Request', parent_request_id = 'parent',
            parent_revision_id = 'parent-revision', parent_head_oid = repeat('b',40),
            review_base_oid = repeat('b',40) WHERE id = 'child-revision';
        INSERT INTO scope_request_revision_context_receipts
            (request_id, operation_key, payload_digest, revision_id, created_at_unix)
            VALUES ('child','context-1',repeat('d',64),'child-revision',2);
        INSERT INTO scope_request_revisions (
            id, request_id, position, actor_user_id, old_head_oid, new_head_oid,
            base_kind, review_base_oid, base_version, main_ancestor_oid, rewrote_history, git_snapshot, created_at_unix
        ) VALUES ('parent-rewrite', 'parent', 2, 'stack-owner', repeat('b',40), repeat('c',40),
            'Main', repeat('a',40), 1, repeat('a',40), TRUE, jsonb_build_object(
                'content_ref', jsonb_build_object('GitBundleSha256', repeat('e',64)),
                'sha256', repeat('e',64), 'git_oid', repeat('c',40),
                'git_file_mode', '100644', 'size_bytes', 1
            ), 2);
        UPDATE scope_requests SET head_oid = repeat('c',40),
            git_snapshot = (SELECT git_snapshot FROM scope_request_revisions WHERE id = 'parent-rewrite'),
            activity_version = 2, closed_at_unix = 2, closed_by_user_id = 'stack-owner', updated_at_unix = 2
            WHERE id = 'parent';
    "#,
    )
    .await
    .unwrap();
    for (sql, constraint) in [
        (
            "UPDATE scope_requests SET parent_head_oid = NULL WHERE id = 'child'",
            "scope_requests_base_values",
        ),
        (
            "UPDATE scope_requests SET review_base_oid = repeat('a',40) WHERE id = 'child'",
            "scope_requests_base_values",
        ),
        (
            "UPDATE scope_requests SET stack_depth = 0 WHERE id = 'parent'",
            "scope_requests_base_values",
        ),
        (
            "UPDATE scope_requests SET stack_depth = 1 WHERE id = 'child'",
            "scope_requests_base_values",
        ),
        (
            "UPDATE scope_requests SET stack_depth = 34 WHERE id = 'child'",
            "scope_requests_base_values",
        ),
        (
            "UPDATE scope_requests SET stack_status = 'main' WHERE id = 'child'",
            "scope_requests_base_values",
        ),
        (
            "UPDATE scope_requests SET parent_revision_id = 'other-revision' WHERE id = 'child'",
            "fk_scope_requests_parent_revision",
        ),
        (
            "UPDATE scope_request_revisions SET parent_head_oid = NULL WHERE id = 'child-revision'",
            "scope_request_revisions_base_values",
        ),
        (
            "UPDATE scope_request_revisions SET parent_revision_id = 'other-revision' WHERE id = 'child-revision'",
            "fk_scope_request_revisions_parent_revision",
        ),
        (
            "DELETE FROM scope_requests WHERE id = 'parent'",
            "fk_scope_requests_parent_revision",
        ),
        (
            "UPDATE scope_request_revision_context_receipts SET revision_id = 'other-revision'",
            "fk_scope_request_revision_context_receipts_revision",
        ),
        (
            "UPDATE scope_request_revision_context_receipts SET created_at_unix = -1",
            "scope_request_revision_context_receipts_values",
        ),
        (
            "INSERT INTO scope_request_revision_context_receipts (request_id,operation_key,payload_digest,revision_id,created_at_unix) VALUES ('child','context-1',repeat('e',64),'child-revision',3)",
            "scope_request_revision_context_receipts_pkey",
        ),
    ] {
        let error = db.execute_unprepared(sql).await.unwrap_err().to_string();
        assert!(error.contains(constraint), "{error}");
    }
    let pinned = db.query_one_raw(Statement::from_string(DatabaseBackend::Postgres,
        "SELECT base_version,parent_request_id,parent_revision_id,parent_head_oid FROM scope_request_revisions WHERE id = 'child-revision'"
    )).await.unwrap().unwrap();
    assert_eq!(pinned.try_get::<i64>("", "base_version").unwrap(), 1);
    assert_eq!(
        pinned.try_get::<String>("", "parent_request_id").unwrap(),
        "parent"
    );
    assert_eq!(
        pinned.try_get::<String>("", "parent_revision_id").unwrap(),
        "parent-revision"
    );
    assert_eq!(
        pinned.try_get::<String>("", "parent_head_oid").unwrap(),
        "b".repeat(40)
    );
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
