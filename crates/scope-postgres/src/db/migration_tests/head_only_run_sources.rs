use super::*;

#[tokio::test]
async fn run_and_trigger_sources_keep_only_their_head_and_retained_segments_are_deleted() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0100_head_only_run_sources")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    let head_oid = "a".repeat(40);
    let frontier = "b".repeat(64);
    let head = serde_json::json!({
        "head_oid": head_oid,
        "push_sequence": 1,
        "change_version": 1,
        "frontier": frontier,
    });
    let spans = serde_json::json!([{
        "first_sequence": 1,
        "last_sequence": 1,
        "geometric_tier": 0,
        "base_oid": null,
        "head_oid": head_oid,
        "segment": {
            "segment_id": "segment-old",
            "sha256": "c".repeat(64),
            "plaintext_bytes": 1,
            "encoding_version": 2,
        },
    }]);
    let source = serde_json::json!({
        "kind": "accepted-git-head",
        "repository_id": "pins-owner/repo",
        "head": head,
        "pack_spans": spans,
    });
    let payload = serde_json::json!({
        "workflow_schema_version": 5,
        "head": head,
        "pack_spans": spans,
        "input": {},
    });
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('pins-owner', 'pins-owner', 'pins-owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id
        ) VALUES (
            'pins-owner/repo', 'pins-owner', 'repo', 'pins-owner', 'Ready', 1, 1, '{}',
            '{"default":"private","rules":[]}', 'repoi_head_only_run_sources'
        );
        INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
        VALUES (repeat('d', 64), '{"jobs":[{"id":"checks"}]}', 1);
        INSERT INTO scope_git_segment_uploads (
            segment_id, repo_id, object_key, state, sha256, plaintext_bytes, encrypted_bytes,
            encoding_version, created_at_unix, updated_at_unix
        ) VALUES (
            'segment-old', 'pins-owner/repo', 'git/segments/v2/pins-owner/repo/segment-old',
            'retained', repeat('c', 64), 1, 2, 2, 1, 1
        );
        INSERT INTO scope_git_segment_references (segment_id, ref_kind, ref_id)
        VALUES ('segment-old', 'run_source', 'run-pinned');
        "#,
    )
    .await
    .unwrap();
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_runs (
            id, idempotency_key, repo_id, workflow_path, workflow_revision_digest, trigger,
            source, state, cancellation_requested, created_at_unix, updated_at_unix
        ) VALUES (
            'run-pinned', 'push-main:1', 'pins-owner/repo', '/.scope/runs/checks.yml',
            repeat('d', 64), 'push-main', $1, 'queued', FALSE, 1, 1
        )",
        [source.into()],
    ))
    .await
    .unwrap();
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_outbox_jobs (
            id, idempotency_key, kind, repo_id, repo_version, payload, state, attempts,
            next_run_at_unix, created_at_unix, updated_at_unix
        ) VALUES (
            'outbox-pinned', 'push_main_trigger_evaluation:pins-owner/repo:1',
            'push_main_trigger_evaluation', 'pins-owner/repo', 1, $1, 'ready', 0, 1, 1, 1
        )",
        [payload.into()],
    ))
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let row = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT
                (SELECT source FROM scope_runs WHERE id = 'run-pinned') AS source,
                (SELECT payload FROM scope_outbox_jobs WHERE id = 'outbox-pinned') AS payload,
                (SELECT state FROM scope_git_segment_uploads WHERE segment_id = 'segment-old')
                    AS segment_state",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<serde_json::Value>("", "source").unwrap(),
        serde_json::json!({
            "kind": "accepted-git-head",
            "repository_id": "pins-owner/repo",
            "head": head,
        })
    );
    assert_eq!(
        row.try_get::<serde_json::Value>("", "payload").unwrap(),
        serde_json::json!({
            "workflow_schema_version": 5,
            "head": head,
            "input": {},
        })
    );
    assert_eq!(
        row.try_get::<String>("", "segment_state").unwrap(),
        "deleting"
    );
    assert!(!relation_exists(&db, "scope_git_segment_references").await);
}

#[tokio::test]
async fn request_bundle_deletion_queues_only_objects_without_run_source_references() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0107_delete_request_bundles")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(r#"
        INSERT INTO scope_users VALUES ('bundle-owner', 'bundle-owner', 'bundle@example.test', TRUE);
        INSERT INTO scope_repositories (id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id)
        VALUES ('bundle-owner/repo', 'bundle-owner', 'repo', 'bundle-owner', 'Ready', 1, 1, '{}', '{}', 'repoi_bundle_deletion');
        INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
        VALUES (repeat('d', 64), '{"jobs":[{"id":"checks"}]}', 1);
    "#).await.unwrap();
    for (id, digest, run_kind) in [
        ("pending", 'a', Some("request-git-snapshot")),
        ("retry", 'b', Some("ephemeral-git-bundle")),
        ("unused", 'c', None),
    ] {
        let blob = scope_domain::content::SourceBlob {
            content_ref: scope_domain::content_ref::ContentRef::git_bundle_sha256(
                digest.to_string().repeat(64),
            ),
            sha256: digest.to_string().repeat(64),
            git_oid: "e".repeat(40),
            git_file_mode: "100644".into(),
            size_bytes: 42,
        };
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "INSERT INTO scope_requests (id, repo_id, name, author_user_id, author_role, audience,
                base_kind, review_base_oid, base_version, stack_status, stack_depth, main_ancestor_oid,
                head_oid, git_snapshot, title, description_markdown, activity_version, created_at_unix, updated_at_unix)
             VALUES ($1, 'bundle-owner/repo', $1, 'bundle-owner', 'Owner', 'private', 'Main', repeat('e',40),
                1, 'main', 1, repeat('e',40), repeat('e',40), $2, '', '', 1, 1, 2)",
            [id.into(), serde_json::to_value(&blob).unwrap().into()])).await.unwrap();
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "INSERT INTO scope_git_fetch_advertisements
                (repo_id, visibility_fingerprint, view_id, reader_id, head_oid, requests, expires_at_unix)
             VALUES ('bundle-owner/repo', 'visibility', 'private', $1, repeat('e',40), $2, 999)",
            [id.into(), serde_json::json!([{"id": id, "view": "private", "author_user_id": "deleted-author", "description_markdown": "private draft", "git_snapshot": blob}]).into()])).await.unwrap();
        crate::db::object_references::insert_object_reference(
            db.as_ref(),
            "request_snapshot",
            id,
            &blob,
        )
        .await
        .unwrap();
        if let Some(kind) = run_kind {
            let source =
                serde_json::json!({"kind": kind, "object": blob, "base_oid": "e".repeat(40)});
            db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
                "INSERT INTO scope_runs (id, idempotency_key, repo_id, workflow_path, workflow_revision_digest,
                    trigger, source, state, cancellation_requested, created_at_unix, updated_at_unix)
                 VALUES ($1, $1, 'bundle-owner/repo', '/.scope/runs/checks.yml', repeat('d',64), 'request', $2, 'queued', FALSE, 1, 1)",
                [id.into(), source.into()])).await.unwrap();
            crate::db::object_references::insert_object_reference(
                db.as_ref(),
                "run_source",
                id,
                &blob,
            )
            .await
            .unwrap();
        }
    }
    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();
    let advertisements = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT reader_id, requests FROM scope_git_fetch_advertisements",
        ))
        .await
        .unwrap();
    assert_eq!(advertisements.len(), 3);
    for row in advertisements {
        let id = row.try_get::<String>("", "reader_id").unwrap();
        assert_eq!(
            row.try_get::<serde_json::Value>("", "requests").unwrap(),
            serde_json::json!([{"id": id, "view": "private"}])
        );
    }
    let queued = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT sha256 FROM scope_orphan_object_jobs",
        ))
        .await
        .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].try_get::<String>("", "sha256").unwrap(),
        "c".repeat(64)
    );
    let references = crate::db::object_references::referenced_content_refs(db.as_ref())
        .await
        .unwrap();
    assert_eq!(
        references,
        ['a', 'b']
            .map(
                |digest| scope_domain::content_ref::ContentRef::git_bundle_sha256(
                    digest.to_string().repeat(64)
                )
            )
            .into()
    );
    let requests = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pushed_at_unix FROM scope_requests",
        ))
        .await
        .unwrap();
    assert!(
        requests
            .iter()
            .all(|row| row.try_get::<i64>("", "pushed_at_unix").unwrap() == 2)
    );
    for table in ["scope_requests", "scope_request_revisions"] {
        assert!(
            db.execute_unprepared(&format!("SELECT git_snapshot FROM {table}"))
                .await
                .is_err()
        );
    }
}
