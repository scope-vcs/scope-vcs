use super::*;

const BASELINE: &str = "m0042_current_schema_baseline";

/// A database that stopped at the baseline: the current schema plus the single
/// baseline ledger entry, with representative business rows and sequence state.
async fn baseline_database(db: &DatabaseConnection) {
    migrations::Migrator::up(db, Some(1)).await.unwrap();
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

async fn sequence_state(db: &DatabaseConnection) -> (i64, bool) {
    let row = db
        .query_one_raw(Statement::from_string(
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
async fn ledgers_outside_the_canonical_prefix_are_rejected_without_changes() {
    // The pre-baseline chain and any unknown entry are equally unrecognizable:
    // this binary only advances a database whose ledger is a prefix of its own.
    for ledger in [
        vec!["m0001_initial_schema", "m0042_request_media"],
        vec![BASELINE, "m9999_unknown"],
        vec!["m0043_retire_git_manifests"],
    ] {
        let (_target, db, _lease) = isolated_database().await;
        baseline_database(&db).await;
        db.execute_unprepared("DELETE FROM seaql_migrations")
            .await
            .unwrap();
        for name in &ledger {
            db.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO seaql_migrations (version, applied_at) VALUES ($1, 1)",
                [(*name).into()],
            ))
            .await
            .unwrap();
        }
        let before = representative_business_snapshot(&db).await;
        let sequence = sequence_state(&db).await;

        for error in [
            migrations::plan(db.as_ref()).await.unwrap_err(),
            migrations::preflight(&db, Default::default())
                .await
                .unwrap_err(),
            migrations::apply_in_maintenance(db.as_ref(), Default::default())
                .await
                .unwrap_err(),
        ] {
            assert!(
                error.to_string().contains(
                    "Scope metadata migration ledger is not a canonical prefix: expected ["
                ),
                "{ledger:?}: {error}"
            );
            assert!(
                error
                    .to_string()
                    .contains(&format!("found [{}]", ledger.join(", "))),
                "{ledger:?}: {error}"
            );
        }
        assert_eq!(applied_versions(&db).await, ledger);
        assert_eq!(representative_business_snapshot(&db).await, before);
        assert_eq!(sequence_state(&db).await, sequence);
    }
}

#[tokio::test]
async fn preflight_refuses_baseline_schema_drift_without_changing_the_ledger() {
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
        baseline_database(&db).await;
        db.execute_unprepared(drift).await.unwrap();
        let ledger = applied_versions(&db).await;
        let before = representative_business_snapshot(&db).await;

        let error = migrations::preflight(&db, Default::default())
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
async fn preflight_checks_the_baseline_schema_with_writers_online_without_changing_data() {
    let (target, db, _lease) = isolated_database().await;
    baseline_database(&db).await;
    let database_url = target.schema_database_url();
    let writer = crate::db::connect_writer_database(&database_url, database_url.parse().unwrap())
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
async fn preflight_rejects_upload_constraint_drift_and_rolls_back_metadata() {
    let (target, db, _lease) = isolated_database().await;
    baseline_database(&db).await;
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
        .query_one_raw(Statement::from_string(DatabaseBackend::Postgres, context))
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
        .query_one_raw(Statement::from_string(DatabaseBackend::Postgres, context))
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

#[tokio::test]
async fn the_visible_public_search_path_survives_the_baseline_schema_check() {
    let (target, db, _lease) = isolated_database().await;
    baseline_database(&db).await;
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
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SHOW search_path",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "search_path")
        .unwrap();

    migrations::preflight(&visible_public, Default::default())
        .await
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
        .query_one_raw(Statement::from_string(
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
