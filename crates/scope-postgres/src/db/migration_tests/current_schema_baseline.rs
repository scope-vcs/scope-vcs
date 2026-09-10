use super::*;

const BASELINE: &str = "m0042_current_schema_baseline";
const ORIGINAL_LEDGER: &str = include_str!("../../migrations/baseline_ledger.txt");

async fn original_chain_database(db: &DatabaseConnection) {
    migrations::Migrator::install(db).await.unwrap();
    db.execute_unprepared(include_str!("fixtures/original_chain_schema.sql"))
        .await
        .unwrap();
    stamp_original_ledger(db, 42).await;
    db.execute_unprepared(
        "INSERT INTO scope_users (id, handle, email, email_verified)
         VALUES ('retained-user', 'retained', 'retained@scope.test', TRUE);
         INSERT INTO scope_auth_identities (provider, subject, user_id)
         VALUES ('clerk', 'retained-subject', 'retained-user');
         INSERT INTO scope_repositories (
             id, owner_handle, name, owner_user_id, publication_state,
             change_version, repo_config, policy, incarnation_id
         ) VALUES (
             'retained-repo', 'retained', 'repo', 'retained-user', 'Ready',
             7, '{}'::jsonb, '{}'::jsonb, 'retained-incarnation'
         );
         SELECT setval('scope_run_creation_sequence', 41, TRUE);",
    )
    .await
    .unwrap();
}

async fn stamp_original_ledger(db: &DatabaseConnection, count: usize) {
    db.execute_unprepared("DELETE FROM seaql_migrations")
        .await
        .unwrap();
    for name in ORIGINAL_LEDGER.lines().take(count) {
        db.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO seaql_migrations (version, applied_at) VALUES ($1, 1)",
            [name.into()],
        ))
        .await
        .unwrap();
    }
}

async fn sequence_state(db: &DatabaseConnection) -> (i64, bool) {
    let row = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT last_value, is_called FROM scope_run_creation_sequence",
        ))
        .await
        .unwrap()
        .unwrap();
    (
        row.try_get("", "last_value").unwrap(),
        row.try_get("", "is_called").unwrap(),
    )
}

#[tokio::test]
async fn original_chain_bridge_preserves_rows_sequences_and_is_idempotent() {
    let (_target, db, _lease) = isolated_database().await;
    original_chain_database(&db).await;
    let before = representative_business_snapshot(&db).await;
    let sequence = sequence_state(&db).await;
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.pending[0].name, BASELINE);
    assert!(!plan.exact);
    assert!(migrations::assert_exact_state(db.as_ref()).await.is_err());
    assert_eq!(applied_versions(&db).await.len(), 42);

    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    assert_eq!(representative_business_snapshot(&db).await, before);
    assert_eq!(sequence_state(&db).await, sequence);
    assert_eq!(applied_versions(&db).await, LATEST_MIGRATIONS);
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    assert_eq!(representative_business_snapshot(&db).await, before);
    assert_eq!(sequence_state(&db).await, sequence);
}

#[tokio::test]
async fn baseline_bridge_preserves_visible_public_extension_operator_classes() {
    let (target, db, _lease) = isolated_database().await;
    original_chain_database(&db).await;
    let before = representative_business_snapshot(&db).await;
    let sequence = sequence_state(&db).await;
    let mut options = sea_orm::ConnectOptions::new(target.schema_database_url());
    options.max_connections(1);
    let visible_public = sea_orm::Database::connect(options).await.unwrap();
    // Deployment uses public, while isolated tests normally hide it. PostgreSQL
    // omits the public qualifier on gin_trgm_ops only when public is visible.
    visible_public
        .execute_unprepared(
            "SELECT set_config('search_path', current_setting('search_path') || ', public', false)",
        )
        .await
        .unwrap();
    let search_path = visible_public
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SHOW search_path",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "search_path")
        .unwrap();

    migrations::apply_in_maintenance(&visible_public, Default::default())
        .await
        .unwrap();

    migrations::assert_exact_state(&visible_public)
        .await
        .unwrap();
    assert_eq!(representative_business_snapshot(&db).await, before);
    assert_eq!(sequence_state(&db).await, sequence);
    let restored_path = visible_public
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SHOW search_path",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "search_path")
        .unwrap();
    assert_eq!(restored_path, search_path);
    visible_public.close().await.unwrap();
}

#[tokio::test]
async fn baseline_bridge_preserves_historical_not_null_constraint_names() {
    let (_target, db, _lease) = isolated_database().await;
    original_chain_database(&db).await;
    // PostgreSQL 18 gives NOT NULL constraints names. Column/table renames in
    // the original chain retained those names without changing the constraint.
    let named_constraint = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT c.conname FROM pg_constraint c
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY(c.conkey)
         WHERE c.conrelid = 'scope_git_segments'::regclass
           AND c.contype = 'n' AND a.attname = 'first_sequence'",
        ))
        .await
        .unwrap();
    if let Some(constraint) = &named_constraint {
        let name = constraint.try_get::<String>("", "conname").unwrap();
        db.execute_unprepared(&format!(
            "ALTER TABLE scope_git_segments RENAME CONSTRAINT \"{}\" TO scope_git_segments_sequence_not_null",
            name.replace('"', "\"\""),
        )).await.unwrap();
    }
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
    if named_constraint.is_some() {
        let retained = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT 1 AS found FROM pg_constraint
             WHERE conrelid = 'scope_git_segments'::regclass
               AND conname = 'scope_git_segments_sequence_not_null'",
            ))
            .await
            .unwrap();
        assert!(
            retained.is_some(),
            "bridging must retain the existing constraint name"
        );
    }
}

#[tokio::test]
async fn failed_baseline_ledger_insert_rolls_back_the_old_ledger_and_data() {
    let (_target, db, _lease) = isolated_database().await;
    original_chain_database(&db).await;
    // This constraint fails after the bridge deletes the old ledger, exercising
    // rollback at the boundary where an interrupted replacement could lose it.
    db.execute_unprepared(
        "ALTER TABLE seaql_migrations ADD CONSTRAINT injected_bridge_failure
         CHECK (version <> 'm0042_current_schema_baseline')",
    )
    .await
    .unwrap();
    let before = representative_business_snapshot(&db).await;
    let ledger = applied_versions(&db).await;
    let error = migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("injected_bridge_failure"),
        "{error}"
    );
    assert_eq!(applied_versions(&db).await, ledger);
    assert_eq!(representative_business_snapshot(&db).await, before);
    assert_eq!(sequence_state(&db).await, (41, true));
    db.execute_unprepared("ALTER TABLE seaql_migrations DROP CONSTRAINT injected_bridge_failure")
        .await
        .unwrap();
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}

#[tokio::test]
async fn retained_older_unknown_and_incomplete_ledgers_are_rejected_without_changes() {
    for count in [33, 41, 42] {
        let (_target, db, _lease) = isolated_database().await;
        original_chain_database(&db).await;
        stamp_original_ledger(&db, count).await;
        if count == 42 {
            db.execute_unprepared("INSERT INTO seaql_migrations VALUES ('m9999_unknown', 1)")
                .await
                .unwrap();
        }
        let ledger = applied_versions(&db).await;
        let before = representative_business_snapshot(&db).await;
        let error = migrations::apply_in_maintenance(db.as_ref(), Default::default())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("578bec00da088598919082b35a7153f62bf0b860")
        );
        assert_eq!(applied_versions(&db).await, ledger);
        assert_eq!(representative_business_snapshot(&db).await, before);
    }
}

#[tokio::test]
async fn baseline_bridge_refuses_schema_drift_without_replacing_the_ledger() {
    for drift in [
        "ALTER TABLE scope_users ALTER COLUMN handle DROP NOT NULL",
        "DROP INDEX idx_scope_runs_history",
        "DROP INDEX idx_scope_run_attempts_expiring; CREATE INDEX idx_scope_run_attempts_expiring ON scope_run_attempts (lease_expires_at_unix, id) WHERE state = 'running'",
        "ALTER TABLE scope_repositories DROP CONSTRAINT scope_repositories_pkey CASCADE",
        "ALTER TABLE scope_git_heads DROP CONSTRAINT scope_git_head_values, ADD CONSTRAINT scope_git_head_values CHECK (push_sequence >= 5 AND change_version >= 0 AND manifest_size_bytes >= 0)",
        "ALTER SEQUENCE scope_run_creation_sequence INCREMENT BY 2",
        "ALTER SEQUENCE scope_run_creation_sequence OWNED BY NONE",
        "CREATE VIEW unknown_view AS SELECT id FROM scope_users",
        "CREATE TABLE unknown_table (id text)",
        "CREATE FUNCTION unknown_function() RETURNS int LANGUAGE SQL AS 'SELECT 1'",
    ] {
        let (_target, db, _lease) = isolated_database().await;
        original_chain_database(&db).await;
        db.execute_unprepared(drift).await.unwrap();
        let ledger = applied_versions(&db).await;
        let before = representative_business_snapshot(&db).await;
        let error = migrations::apply_in_maintenance(db.as_ref(), Default::default())
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("schema drift"),
            "{drift}: {error}"
        );
        assert_eq!(applied_versions(&db).await, ledger);
        assert_eq!(representative_business_snapshot(&db).await, before);
    }
}

#[tokio::test]
async fn baseline_initialization_refuses_an_untracked_nonempty_schema() {
    let (_target, db, _lease) = isolated_database().await;
    db.execute_unprepared(
        "CREATE TABLE retained_unknown (id text); INSERT INTO retained_unknown VALUES ('keep')",
    )
    .await
    .unwrap();
    let error = migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("empty schema"));
    assert!(relation_exists(&db, "retained_unknown").await);
    assert!(!relation_exists(&db, "seaql_migrations").await);
}

#[tokio::test]
async fn preflight_checks_retained_schema_with_writers_online_without_changing_data() {
    let (target, db, _lease) = isolated_database().await;
    original_chain_database(&db).await;
    let writer = crate::db::connect_writer_database(&target.schema_database_url())
        .await
        .unwrap();
    let before = representative_business_snapshot(&db).await;
    let sequence = sequence_state(&db).await;
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(
        migrations::preflight(&db, Default::default())
            .await
            .unwrap(),
        plan
    );
    assert_eq!(representative_business_snapshot(&db).await, before);
    assert_eq!(sequence_state(&db).await, sequence);
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), plan);
    writer.ping().await.unwrap();
    writer.close().await.unwrap();
}

#[tokio::test]
async fn preflight_rejects_retained_upload_constraint_drift_and_rolls_back_metadata() {
    let (target, db, _lease) = isolated_database().await;
    original_chain_database(&db).await;
    db.execute_unprepared(
        "ALTER TABLE scope_git_segment_uploads
         DROP CONSTRAINT scope_git_segment_upload_state,
         DROP CONSTRAINT scope_git_segment_upload_values,
         ADD CONSTRAINT scope_git_segment_upload_state CHECK
             (state IN ('uploading', 'ready', 'published', 'deleting', 'deleted')),
         ADD CONSTRAINT scope_git_segment_upload_values CHECK (
             length(btrim(segment_id)) > 0 AND length(btrim(object_key)) > 0
             AND encoding_version > 0 AND created_at_unix >= 0
             AND updated_at_unix >= created_at_unix
             AND (sha256 IS NULL OR length(sha256) = 64)
             AND (plaintext_bytes IS NULL OR plaintext_bytes >= 0)
             AND (encrypted_bytes IS NULL OR encrypted_bytes >= 0)
             AND (state NOT IN ('ready', 'published') OR
                  (sha256 IS NOT NULL AND plaintext_bytes IS NOT NULL AND encrypted_bytes IS NOT NULL)))",
    )
    .await
    .unwrap();
    let mut options = sea_orm::ConnectOptions::new(target.schema_database_url());
    options.max_connections(1);
    let connection = sea_orm::Database::connect(options).await.unwrap();
    let context = "SELECT current_setting('search_path') AS path,
        (SELECT count(*)::bigint FROM pg_namespace WHERE nspname LIKE 'scope_baseline_check_%') AS comparisons";
    let before = connection
        .query_one(Statement::from_string(DatabaseBackend::Postgres, context))
        .await
        .unwrap()
        .unwrap();
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    let snapshot = representative_business_snapshot(&db).await;
    let error = migrations::preflight(&connection, Default::default())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("scope_git_segment_uploads"), "{error}");
    assert!(error.contains("scope_git_segment_upload_values"), "{error}");
    assert!(error.contains("retained"), "{error}");
    assert!(error.contains("expected expressions"), "{error}");
    let after = connection
        .query_one(Statement::from_string(DatabaseBackend::Postgres, context))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        before.try_get::<String>("", "path").unwrap(),
        after.try_get::<String>("", "path").unwrap()
    );
    assert_eq!(
        before.try_get::<i64>("", "comparisons").unwrap(),
        after.try_get::<i64>("", "comparisons").unwrap()
    );
    assert!(!relation_exists(&connection, "pg_temp.scope_baseline_expression_inventory").await);
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), plan);
    assert_eq!(representative_business_snapshot(&db).await, snapshot);
    connection.close().await.unwrap();
}
