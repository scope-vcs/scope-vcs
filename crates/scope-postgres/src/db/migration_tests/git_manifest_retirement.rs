use super::isolated_database;
use crate::migrations;
use scope_domain::runs::source::RunSource;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use sea_orm_migration::MigratorTrait;

#[tokio::test]
async fn manifest_retirement_preserves_frontiers_pins_and_enqueues_shared_objects_once() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('cutover_user', 'cutover-user', 'cutover@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy, incarnation_id
        ) VALUES (
            'cutover-user/repo', 'cutover-user', 'repo', 'cutover_user', 'Ready', 1,
            '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}',
            '{"default_visibility":"Private","rules":[]}', 'repoi_manifest_test'
        );
        INSERT INTO scope_git_segment_uploads (
            segment_id, repo_id, object_key, state, sha256, plaintext_bytes,
            encrypted_bytes, encoding_version, created_at_unix, updated_at_unix
        ) VALUES (
            'segment-test', 'cutover-user/repo', 'git/segments/v2/segment-test',
            'published', repeat('a',64), 3, 99, 2, 1, 1
        );
        INSERT INTO scope_git_segments (
            repo_id, first_sequence, last_sequence, geometric_tier,
            base_oid, head_oid, segment_id
        ) VALUES ('cutover-user/repo', 1, 1, 0, NULL, repeat('b',40), 'segment-test');
        INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
        VALUES (repeat('c',64), '{"jobs":[{}]}', 1);
        INSERT INTO scope_runs (
            id, idempotency_key, repo_id, workflow_path, workflow_revision_digest,
            trigger, requested_by_user_id, source, state, cancellation_requested,
            created_at_unix, updated_at_unix, completed_at_unix
        ) VALUES (
            'pinned-run', 'pinned-run', 'cutover-user/repo', '.scope/runs/checks.yml',
            repeat('c',64), 'manual', 'cutover_user',
            jsonb_build_object(
                'kind', 'accepted-git-head', 'repository_id', 'cutover-user/repo',
                'audience', 'Private',
                'head', jsonb_build_object(
                    'head_oid', repeat('b',40), 'push_sequence', 1, 'change_version', 1,
                    'manifest', jsonb_build_object(
                        'content_ref', jsonb_build_object('GitManifestSha256', repeat('d',64)),
                        'sha256', repeat('d',64), 'git_oid', repeat('b',40),
                        'git_file_mode', '100644', 'size_bytes', 4
                    )
                ),
                'pack_spans', jsonb_build_array(jsonb_build_object(
                    'first_sequence', 1, 'last_sequence', 1, 'geometric_tier', 0,
                    'base_oid', NULL, 'head_oid', repeat('b',40),
                    'segment', jsonb_build_object('segment_id', 'segment-test',
                        'sha256', repeat('a',64), 'plaintext_bytes', 3, 'encoding_version', 2)
                ))
            ), 'succeeded', FALSE, 1, 2, 2
        );
        INSERT INTO scope_git_segment_references (segment_id, ref_kind, ref_id)
        VALUES ('segment-test', 'run_source', 'pinned-run'),
               ('segment-test', 'push_trigger_source', 'cutover-user/repo:1');
        INSERT INTO scope_outbox_jobs (
            id, idempotency_key, kind, repo_id, repo_version, payload,
            state, attempts, next_run_at_unix, created_at_unix, updated_at_unix
        ) VALUES ('pending-push', 'pending-push', 'push_main_trigger_evaluation',
                  'cutover-user/repo', 1, '{"workflow_schema_version":5}', 'ready', 0, 1, 1, 1);
        INSERT INTO scope_git_heads (
            repo_id, head_oid, push_sequence, change_version,
            manifest_object_key, manifest_sha256, manifest_size_bytes
        ) VALUES (
            'cutover-user/repo', repeat('b', 40), 1, 1,
            jsonb_build_object('GitManifestSha256', repeat('d', 64))::text,
            repeat('d', 64), 4
        );
        UPDATE scope_outbox_jobs SET payload = jsonb_set(payload, '{head}',
            (SELECT source->'head' FROM scope_runs WHERE id = 'pinned-run'))
        WHERE id = 'pending-push';
        INSERT INTO scope_orphan_object_jobs (
            object_key, generation, sha256, git_oid, size_bytes, attempts,
            next_run_at_unix, created_at_unix, updated_at_unix
        ) VALUES (
            jsonb_build_object('GitManifestSha256', repeat('f',64))::text,
            'old-migration', repeat('f',64), repeat('b',40), 8, 2, 9, 1, 2
        );
        INSERT INTO scope_object_references (object_key, ref_kind, ref_id)
        VALUES
            (jsonb_build_object('GitManifestSha256', repeat('d', 64))::text,
             'run_source', 'pinned-run'),
            (jsonb_build_object('GitManifestSha256', repeat('d', 64))::text,
             'git_manifest', 'cutover-user/repo'),
            (jsonb_build_object('GitManifestSha256', repeat('d', 64))::text,
             'push_trigger_source', 'cutover-user/repo:1'),
            (jsonb_build_object('BlobSha256', repeat('e', 64))::text,
             'run_source', 'unrelated');
        "#,
    )
    .await
    .unwrap();
    let before = json_column(
        db.as_ref(),
        "SELECT source AS value FROM scope_runs WHERE id = 'pinned-run'",
    )
    .await;
    let pins_before = json_column(db.as_ref(),
        "SELECT jsonb_agg(to_jsonb(refs) ORDER BY ref_kind, ref_id) AS value FROM scope_git_segment_references refs").await;
    let spans_before = json_column(
        db.as_ref(),
        "SELECT jsonb_agg(to_jsonb(spans)) AS value FROM scope_git_segments spans",
    )
    .await;
    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();
    let after = json_column(
        db.as_ref(),
        "SELECT source AS value FROM scope_runs WHERE id = 'pinned-run'",
    )
    .await;
    let mut expected = before.clone();
    expected["head"]["frontier"] = before["head"]["manifest"]["sha256"].clone();
    expected["head"].as_object_mut().unwrap().remove("manifest");
    assert_eq!(after, expected);
    let source = serde_json::from_value::<RunSource>(after.clone()).unwrap();
    assert_eq!(source.source_identity(), "d".repeat(64));
    assert_eq!(
        source.logical_git_head().unwrap().1.frontier.digest(),
        "d".repeat(64)
    );
    assert_eq!(json_column(db.as_ref(),
        "SELECT to_jsonb(frontier_digest) AS value FROM scope_git_heads WHERE repo_id = 'cutover-user/repo'").await, serde_json::json!("d".repeat(64)));
    let payload = json_column(
        db.as_ref(),
        "SELECT payload AS value FROM scope_outbox_jobs WHERE id = 'pending-push'",
    )
    .await;
    assert_eq!(payload["head"], after["head"]);
    assert_eq!(pins_before, json_column(db.as_ref(),
        "SELECT jsonb_agg(to_jsonb(refs) ORDER BY ref_kind, ref_id) AS value FROM scope_git_segment_references refs").await);
    assert_eq!(
        spans_before,
        json_column(
            db.as_ref(),
            "SELECT jsonb_agg(to_jsonb(spans)) AS value FROM scope_git_segments spans"
        )
        .await
    );
    assert!(!column_exists(db.as_ref(), "scope_git_heads", "manifest_object_key").await);
    assert_eq!(scalar_i64(db.as_ref(),
        "SELECT count(*) AS value FROM scope_object_references WHERE object_key::jsonb ? 'GitManifestSha256'").await, 0);
    assert_eq!(scalar_i64(db.as_ref(),
        "SELECT count(*) AS value FROM scope_object_references WHERE object_key::jsonb ? 'BlobSha256'").await, 1);
    assert_eq!(scalar_i64(db.as_ref(),
        "SELECT count(*) AS value FROM scope_orphan_object_jobs WHERE object_key::jsonb ? 'GitManifestSha256' AND completed_at_unix IS NULL AND size_bytes = 4").await, 1);
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    assert_eq!(scalar_i64(db.as_ref(),
        "SELECT count(*) AS value FROM scope_orphan_object_jobs WHERE object_key::jsonb ? 'GitManifestSha256'").await, 2);
}

async fn json_column(db: &DatabaseConnection, sql: &str) -> serde_json::Value {
    db.query_one(Statement::from_string(
        DatabaseBackend::Postgres,
        sql.to_string(),
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get("", "value")
    .unwrap()
}

async fn scalar_i64(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one(Statement::from_string(
        DatabaseBackend::Postgres,
        sql.to_string(),
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get("", "value")
    .unwrap()
}

async fn column_exists(db: &DatabaseConnection, table: &str, column: &str) -> bool {
    db.query_one(Statement::from_sql_and_values(DatabaseBackend::Postgres,
        "SELECT 1 FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = $1 AND column_name = $2",
        [table.into(), column.into()])).await.unwrap().is_some()
}
