use super::*;

#[tokio::test]
async fn fresh_database_reaches_exact_latest_schema() {
    let (_target, db, _lease) = isolated_database().await;

    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();

    migrations::assert_exact_state(db.as_ref()).await.unwrap();
    assert_eq!(applied_versions(db.as_ref()).await, LATEST_MIGRATIONS);
    assert!(!relation_exists(db.as_ref(), "scope_metadata_schema").await);
    assert!(!relation_exists(db.as_ref(), "scope_metadata_reset_events").await);

    let projection_columns = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "
                SELECT
                    bool_and(is_nullable = 'YES') FILTER (WHERE column_name = 'head_oid') AS nullable_head,
                    bool_and(is_nullable = 'NO') FILTER (WHERE column_name = 'identity_version') AS required_identity
                FROM information_schema.columns
                WHERE table_schema = current_schema()
                  AND table_name = 'scope_projection_read_models'
                  AND column_name IN ('head_oid', 'identity_version')
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert!(
        projection_columns
            .try_get::<bool>("", "nullable_head")
            .unwrap()
    );
    assert!(
        projection_columns
            .try_get::<bool>("", "required_identity")
            .unwrap()
    );
    let attempt_columns = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT column_name FROM information_schema.columns
         WHERE table_schema = current_schema() AND table_name = 'scope_run_attempts'
           AND column_name IN ('execution_provider', 'runner_stop_claimed_at_unix',
                               'runner_stop_completed_at_unix')
         ORDER BY column_name",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "column_name").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        attempt_columns,
        [
            "runner_stop_claimed_at_unix",
            "runner_stop_completed_at_unix"
        ]
    );
    let scope_tables = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT tablename FROM pg_tables
             WHERE schemaname = current_schema() AND left(tablename, 6) = 'scope_'
             ORDER BY tablename",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "tablename").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(scope_tables.len(), 68);
    assert!(relation_exists(db.as_ref(), "scope_request_ref_cleanup_jobs").await);
    assert!(relation_exists(db.as_ref(), "scope_repository_history_views").await);
    assert!(relation_exists(db.as_ref(), "scope_repository_history_entries").await);
    assert!(relation_exists(db.as_ref(), "scope_git_segment_uploads").await);
    assert!(relation_exists(db.as_ref(), "scope_cache_orphan_uploads").await);
    assert!(relation_exists(db.as_ref(), "scope_run_attempt_cache_setups").await);
    assert!(!relation_exists(db.as_ref(), "scope_user_credit_accounts").await);
    assert!(!relation_exists(db.as_ref(), "scope_credit_ledger_entries").await);
    let review_columns = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "
                SELECT count(*) AS count
                FROM information_schema.columns
                WHERE table_schema = current_schema()
                  AND table_name = 'scope_requests'
                  AND column_name IN (
                    'held_at_unix', 'held_by_user_id', 'assessment_outcome',
                    'assessment_body_markdown', 'assessed_at_unix', 'assessed_by_user_id'
                  )
            "
            .to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(review_columns, 0);
}
