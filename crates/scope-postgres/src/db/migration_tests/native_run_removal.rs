use super::*;

#[tokio::test]
async fn removing_native_runs_releases_their_sources_and_reevaluates_their_requests() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0111_remove_native_runs")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    let blob = scope_domain::content::SourceBlob {
        content_ref: scope_domain::content_ref::ContentRef::git_bundle_sha256("a".repeat(64)),
        sha256: "a".repeat(64),
        git_oid: "b".repeat(40),
        git_file_mode: "100644".into(),
        size_bytes: 42,
    };
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users VALUES ('owner', 'owner', 'owner@example.test', TRUE);
        INSERT INTO scope_repositories (id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id)
        VALUES ('owner/repo', 'owner', 'repo', 'owner', 'Ready', 1, 1, '{}', '{}', 'repoi_native_removal');
        INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
        VALUES (repeat('d', 64), '{"jobs":[{"id":"checks"}]}', 1);
        INSERT INTO scope_outbox_jobs (id, idempotency_key, kind, repo_id, repo_version, payload,
            state, attempts, next_run_at_unix, created_at_unix, updated_at_unix)
        VALUES ('trigger', 'trigger', 'push_main_trigger_evaluation', 'owner/repo', 1,
            '{"workflow_schema_version": 5}', 'ready', 0, 1, 1, 1);
        "#,
    )
    .await
    .unwrap();
    for (request_id, checks) in [
        (
            "native",
            serde_json::json!([{
                "provider": "native",
                "workflow_path": "/.scope/runs/checks.yml",
                "workflow_name": "checks",
                "workflow_revision_digest": "d".repeat(64),
                "run_id": "run_native",
            }]),
        ),
        (
            "github",
            serde_json::json!([
                {"provider": "github", "name": "ci / test"},
                {"provider": "github", "name": "ci / lint"},
            ]),
        ),
    ] {
        for (sql, values) in [
            (
                "INSERT INTO scope_requests (id, repo_id, name, author_user_id, author_role, audience,
                    base_kind, review_base_oid, base_version, stack_status, stack_depth,
                    main_ancestor_oid, head_oid, title, description_markdown, activity_version,
                    created_at_unix, updated_at_unix)
                 VALUES ($1, 'owner/repo', $1, 'owner', 'Owner', 'private', 'Main', repeat('e',40),
                    1, 'main', 1, repeat('e',40), repeat('b',40), '', '', 1, 1, 2)",
                vec![request_id.into()],
            ),
            (
                "INSERT INTO scope_request_revisions (id, request_id, position, actor_user_id,
                    old_head_oid, new_head_oid, main_ancestor_oid, base_kind, review_base_oid,
                    base_version, rewrote_history, created_at_unix)
                 VALUES ($1, $1, 1, 'owner', repeat('e',40), repeat('b',40), repeat('e',40),
                    'Main', repeat('e',40), 1, FALSE, 2)",
                vec![request_id.into()],
            ),
            (
                "INSERT INTO scope_request_check_evaluations (id, revision_id, base_version,
                    review_base_oid, request_id, head_oid, tested_oid, state, message, checks,
                    created_at_unix, updated_at_unix)
                 VALUES ($1, $1, 1, repeat('e',40), $1, repeat('b',40), repeat('b',40), 'started',
                    NULL, $2, 2, 2)",
                vec![request_id.into(), checks.clone().into()],
            ),
            (
                "UPDATE scope_requests SET current_check_evaluation_id = $1 WHERE id = $1",
                vec![request_id.into()],
            ),
        ] {
            db.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                sql,
                values,
            ))
            .await
            .unwrap();
        }
    }
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_runs (id, idempotency_key, repo_id, workflow_path, workflow_revision_digest,
            trigger, source, state, cancellation_requested, created_at_unix, updated_at_unix)
         VALUES ('run_native', 'run_native', 'owner/repo', '/.scope/runs/checks.yml', repeat('d',64),
            'request', $1, 'queued', FALSE, 1, 1)",
        [serde_json::json!({"kind": "ephemeral-git-bundle", "object": blob}).into()],
    ))
    .await
    .unwrap();
    crate::db::object_references::insert_object_reference(
        db.as_ref(),
        "run_source",
        "run_native",
        &blob,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let requests = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT request.id, request.current_check_evaluation_id, evaluation.checks
             FROM scope_requests request
             LEFT JOIN scope_request_check_evaluations evaluation
                ON evaluation.id = request.current_check_evaluation_id
             ORDER BY request.id",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "id").unwrap(),
                row.try_get::<Option<String>>("", "current_check_evaluation_id")
                    .unwrap(),
                row.try_get::<Option<serde_json::Value>>("", "checks")
                    .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        requests,
        [
            (
                "github".to_string(),
                Some("github".to_string()),
                Some(serde_json::json!([{"name": "ci / test"}, {"name": "ci / lint"}])),
            ),
            ("native".to_string(), None, None),
        ]
    );
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
        "a".repeat(64)
    );
    assert!(
        crate::db::object_references::referenced_content_refs(db.as_ref())
            .await
            .unwrap()
            .is_empty()
    );
    let outbox = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_outbox_jobs",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbox.try_get::<i64>("", "count").unwrap(), 0);
    for dropped in [
        "scope_runs",
        "scope_run_logs",
        "scope_workflow_revisions",
        "scope_repository_workflow_catalogs",
        "scope_push_trigger_evaluations",
        "scope_native_runs_accounts",
        "scope_cache_objects",
    ] {
        assert!(!relation_exists(&db, dropped).await, "{dropped}");
    }
    assert!(
        db.execute_unprepared(
            "UPDATE scope_request_check_evaluations
             SET checks = '[{\"provider\": \"github\"}]'"
        )
        .await
        .is_err()
    );
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
