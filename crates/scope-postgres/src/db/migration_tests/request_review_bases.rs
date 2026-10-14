use super::*;

#[tokio::test]
async fn review_context_backfill_preserves_saved_revision_bases_and_only_links_current_evidence() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0084_request_review_bases")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('base-owner', 'base-owner', 'base-owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id
        ) VALUES (
            'base-owner/repo', 'base-owner', 'repo', 'base-owner', 'Ready', 1, 1,
            '{}', '{}', 'repoi_review_base_backfill'
        );
        INSERT INTO scope_requests (
            id, repo_id, name, author_user_id, author_role, audience,
            base_main_oid, head_oid, title, description_markdown, activity_version,
            created_at_unix, updated_at_unix
        ) SELECT name, 'base-owner/repo', name, 'base-owner', 'Owner', 'private',
                 repeat('c', 40), repeat('d', 40), name, '', 2, 1, 3
          FROM (VALUES ('current'), ('older-base'), ('orphan'), ('legacy-empty')) AS fixture(name);
        INSERT INTO scope_request_revisions (
            id, request_id, position, actor_user_id, old_head_oid, new_head_oid,
            base_main_oid, rewrote_history, git_snapshot, created_at_unix
        ) VALUES
            ('current-old', 'current', 1, 'base-owner', repeat('a', 40), repeat('d', 40),
             repeat('a', 40), FALSE, '{"saved":"original"}', 2),
            ('current-latest', 'current', 2, 'base-owner', repeat('b', 40), repeat('d', 40),
             repeat('c', 40), FALSE, '{"saved":"latest"}', 3),
            ('older-base-revision', 'older-base', 2, 'base-owner', repeat('a', 40), repeat('d', 40),
             repeat('a', 40), TRUE, '{"saved":"older-base"}', 3),
            ('legacy-empty-revision', 'legacy-empty', 1, 'base-owner', repeat('c', 40), repeat('d', 40),
             repeat('c', 40), FALSE, '{"saved":"empty"}', 3);
        INSERT INTO scope_request_check_evaluations (
            request_id, head_oid, tested_oid, state, message, checks,
            created_at_unix, updated_at_unix
        ) SELECT name, repeat('d', 40), repeat('d', 40),
                 CASE WHEN name IN ('current', 'older-base') THEN 'started' ELSE 'no-checks' END,
                 NULL,
                 CASE WHEN name IN ('current', 'older-base') THEN jsonb_build_array(jsonb_build_object(
                    'provider', 'native', 'workflow_path', '/.scope/runs/checks.yml',
                    'workflow_name', 'checks', 'workflow_revision_digest', repeat('e', 64), 'run_id', 'run-check'
                 )) ELSE '[]'::jsonb END, 3, 3
          FROM (VALUES ('current'), ('older-base'), ('orphan'), ('legacy-empty')) AS fixture(name);
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let revisions = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, base_kind, review_base_oid, main_ancestor_oid, rewrote_history, git_snapshot
             FROM scope_request_revisions ORDER BY id",
        ))
        .await
        .unwrap();
    assert_eq!(revisions.len(), 4);
    for (row, (expected_base, expected_rewrite, expected_snapshot)) in revisions.iter().zip([
        ('c', false, "latest"),
        ('a', false, "original"),
        ('c', false, "empty"),
        ('a', true, "older-base"),
    ]) {
        assert_eq!(row.try_get::<String>("", "base_kind").unwrap(), "Main");
        assert_eq!(
            row.try_get::<String>("", "review_base_oid").unwrap(),
            expected_base.to_string().repeat(40)
        );
        assert_eq!(
            row.try_get::<String>("", "main_ancestor_oid").unwrap(),
            expected_base.to_string().repeat(40)
        );
        assert_eq!(
            row.try_get::<bool>("", "rewrote_history").unwrap(),
            expected_rewrite
        );
        assert_eq!(
            row.try_get::<serde_json::Value>("", "git_snapshot")
                .unwrap(),
            serde_json::json!({ "saved": expected_snapshot })
        );
    }
    let evaluations = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, request_id, revision_id, review_base_oid, base_version
             FROM scope_request_check_evaluations ORDER BY request_id",
        ))
        .await
        .unwrap();
    assert_eq!(evaluations.len(), 3);
    for (row, (request_id, revision_id, base)) in evaluations.iter().zip([
        ("current", "current-latest", 'c'),
        ("legacy-empty", "legacy-empty-revision", 'c'),
        ("older-base", "older-base-revision", 'a'),
    ]) {
        assert_eq!(row.try_get::<String>("", "request_id").unwrap(), request_id);
        assert_eq!(
            row.try_get::<String>("", "revision_id").unwrap(),
            revision_id
        );
        assert_eq!(
            row.try_get::<String>("", "review_base_oid").unwrap(),
            base.to_string().repeat(40)
        );
        assert_eq!(row.try_get::<i64>("", "base_version").unwrap(), 1);
    }
    let requests = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT request.id, request.base_kind, request.main_ancestor_oid,
                    request.review_base_oid, request.base_version,
                    evaluation.revision_id AS current_revision_id
             FROM scope_requests request
             LEFT JOIN scope_request_check_evaluations evaluation
               ON evaluation.id = request.current_check_evaluation_id
             ORDER BY request.id",
        ))
        .await
        .unwrap();
    assert_eq!(requests.len(), 4);
    for (row, expected_revision) in requests
        .iter()
        .zip([Some("current-latest"), None, None, None])
    {
        assert_eq!(row.try_get::<String>("", "base_kind").unwrap(), "Main");
        assert_eq!(
            row.try_get::<String>("", "main_ancestor_oid").unwrap(),
            "c".repeat(40)
        );
        assert_eq!(
            row.try_get::<String>("", "review_base_oid").unwrap(),
            "c".repeat(40)
        );
        assert_eq!(row.try_get::<i64>("", "base_version").unwrap(), 1);
        assert_eq!(
            row.try_get::<Option<String>>("", "current_revision_id")
                .unwrap()
                .as_deref(),
            expected_revision
        );
    }
    let context = scope_domain::requests::RequestCheckContext::new(
        "current",
        "current-latest",
        "d".repeat(40),
        1,
        "c".repeat(40),
    )
    .unwrap();
    let expected_evaluation = scope_domain::requests::RequestCheckEvaluation::started(
        &context,
        vec![scope_domain::requests::RequestCheck::Native(
            scope_domain::requests::NativeRequestCheck {
                workflow_path: "/.scope/runs/checks.yml".into(),
                workflow_name: "checks".into(),
                workflow_revision_digest: "e".repeat(64),
                run_id: Some("run-check".into()),
            },
        )],
        3,
    )
    .unwrap();
    assert_eq!(
        evaluations[0].try_get::<String>("", "id").unwrap(),
        expected_evaluation.id
    );
    db.execute_unprepared("DELETE FROM scope_requests WHERE id = 'current'")
        .await
        .unwrap();
    let remaining = db.query_one_raw(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT count(*) AS count FROM scope_request_check_evaluations WHERE request_id = 'current'",
    )).await.unwrap().unwrap();
    assert_eq!(remaining.try_get::<i64>("", "count").unwrap(), 0);
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
