use super::*;
use sea_orm::TransactionTrait;

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
    for id in ["parent", "child"] {
        db.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_requests SET stack_status = 'needs_main_update' WHERE id = $1",
            [id.into()],
        ))
        .await
        .unwrap();
        let status = db
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT stack_status FROM scope_requests WHERE id = $1",
                [id.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<String>("", "stack_status")
            .unwrap();
        assert_eq!(status, "needs_main_update");
    }
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
    let tx = db.begin().await.unwrap();
    tx.execute_unprepared("SET LOCAL enable_seqscan = off; SET LOCAL enable_bitmapscan = off;")
        .await
        .unwrap();
    for (lifecycle, transition) in [
        (
            "closed",
            "closed_at_unix = 2, closed_by_user_id = 'stack-owner'",
        ),
        (
            "merged",
            "closed_at_unix = NULL, closed_by_user_id = NULL, merged_at_unix = 2, merged_by_user_id = 'stack-owner', merged_head_oid = repeat('b',40), merged_main_oid = repeat('b',40)",
        ),
    ] {
        tx.execute_unprepared(&format!(
            "UPDATE scope_requests SET {transition}, submitted_at_unix = 1, updated_at_unix = 2 WHERE id = 'child'"
        )).await.unwrap();
        let children = tx.query_all_raw(Statement::from_string(DatabaseBackend::Postgres,
            "SELECT id FROM scope_requests WHERE repo_id = 'stack-owner/repo' AND parent_request_id = 'parent' AND audience = 'private' AND id > 'a' ORDER BY id LIMIT 2"
        )).await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].try_get::<String>("", "id").unwrap(), "child");
        let plan = tx.query_one_raw(Statement::from_string(DatabaseBackend::Postgres,
            "EXPLAIN (FORMAT JSON) SELECT id FROM scope_requests WHERE repo_id = 'stack-owner/repo' AND parent_request_id = 'parent' AND audience = 'private' AND id > 'a' ORDER BY id LIMIT 2"
        )).await.unwrap().unwrap().try_get::<serde_json::Value>("", "QUERY PLAN").unwrap();
        let scan = &plan[0]["Plan"]["Plans"][0];
        assert!(
            matches!(
                scan["Node Type"].as_str(),
                Some("Index Scan" | "Index Only Scan")
            ),
            "{lifecycle}: {plan}"
        );
        let condition = scan["Index Cond"].as_str().unwrap();
        for key in ["repo_id", "parent_request_id", "audience", "> 'a'"] {
            assert!(condition.contains(key), "{lifecycle}: {plan}");
        }
    }
    tx.commit().await.unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}

#[tokio::test]
async fn stack_migration_backfills_revision_versions_from_ordered_base_transitions() {
    let (_target, db, _lease) = isolated_database().await;
    let review_base_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0084_request_review_bases")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(review_base_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('version-owner', 'version-owner', 'versions@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id
        ) VALUES ('version-owner/repo', 'version-owner', 'repo', 'version-owner',
            'Ready', 1, 1, '{}', '{}', 'repoi_revision_versions');
        INSERT INTO scope_requests (
            id, repo_id, name, author_user_id, author_role, audience,
            base_main_oid, head_oid, title, description_markdown, activity_version,
            created_at_unix, updated_at_unix
        ) SELECT name, 'version-owner/repo', name, 'version-owner', 'Owner', 'private',
            repeat('a',40), repeat('f',40), name, '', 6, 1, 6
          FROM (VALUES ('history'), ('first-change'), ('legacy'), ('draft')) AS fixture(name);
        INSERT INTO scope_request_revisions (
            id, request_id, position, actor_user_id, old_head_oid, new_head_oid,
            base_main_oid, rewrote_history, git_snapshot, created_at_unix
        ) SELECT 'legacy-' || position, 'legacy', position, 'version-owner',
            repeat('f',40), repeat('f',40), repeat(base,40), FALSE, '{}', position
          FROM (VALUES (1,'b'), (2,'a')) AS fixture(position,base);
    "#,
    )
    .await
    .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(2))
        .await
        .unwrap();
    db.execute_unprepared(r#"
        UPDATE scope_requests SET base_version = CASE id
            WHEN 'history' THEN 3 WHEN 'first-change' THEN 2 ELSE 1 END;
        UPDATE scope_requests SET review_base_oid = repeat('b',40), main_ancestor_oid = repeat('b',40)
            WHERE id = 'first-change';
        INSERT INTO scope_request_revisions (
            id, request_id, position, actor_user_id, old_head_oid, new_head_oid,
            base_kind, review_base_oid, main_ancestor_oid, rewrote_history, git_snapshot, created_at_unix
        ) SELECT 'history-' || position, 'history', position, 'version-owner',
            repeat('f',40), repeat('f',40), 'Main', repeat(base,40), repeat(base,40),
            rewrote, '{}', position
          FROM (VALUES (1,'a',FALSE), (2,'a',TRUE), (3,'b',TRUE),
            (4,'b',FALSE), (5,'a',TRUE), (6,'a',FALSE)) AS fixture(position,base,rewrote);
        INSERT INTO scope_request_revisions (
            id, request_id, position, actor_user_id, old_head_oid, new_head_oid,
            base_kind, review_base_oid, main_ancestor_oid, rewrote_history, git_snapshot, created_at_unix
        ) VALUES ('first-change-1', 'first-change', 1, 'version-owner', repeat('a',40),
            repeat('f',40), 'Main', repeat('b',40), repeat('b',40), TRUE, '{}', 1);
    "#).await.unwrap();
    migrations::Migrator::up(db.as_ref(), None).await.unwrap();
    let versions = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, base_version FROM scope_request_revisions ORDER BY request_id, position",
        ))
        .await
        .unwrap();
    let actual: Vec<(String, i64)> = versions
        .iter()
        .map(|row| {
            (
                row.try_get("", "id").unwrap(),
                row.try_get("", "base_version").unwrap(),
            )
        })
        .collect();
    assert_eq!(
        actual,
        [
            ("first-change-1".into(), 2),
            ("history-1".into(), 1),
            ("history-2".into(), 1),
            ("history-3".into(), 2),
            ("history-4".into(), 2),
            ("history-5".into(), 3),
            ("history-6".into(), 3),
            ("legacy-1".into(), 1),
            ("legacy-2".into(), 1),
        ]
    );
    let requests = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, base_version FROM scope_requests ORDER BY id",
        ))
        .await
        .unwrap();
    let request_versions: Vec<(String, i64)> = requests
        .iter()
        .map(|row| {
            (
                row.try_get("", "id").unwrap(),
                row.try_get("", "base_version").unwrap(),
            )
        })
        .collect();
    assert_eq!(
        request_versions,
        [
            ("draft".into(), 1),
            ("first-change".into(), 2),
            ("history".into(), 3),
            ("legacy".into(), 1),
        ]
    );
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
