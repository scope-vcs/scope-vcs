use super::*;

#[tokio::test]
async fn run_and_trigger_sources_keep_only_their_head_and_retained_segments_are_deleted() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0094_head_only_run_sources")
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

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

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
