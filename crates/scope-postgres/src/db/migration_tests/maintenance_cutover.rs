use super::*;
use crate::db::{
    apply_maintenance_migrations, connect_postgres_store, connect_postgres_worker_store,
    connect_writer_database, terminate_metadata_writer_sessions, verify_writer_fence_available,
};

#[tokio::test]
async fn ordinary_startup_refuses_pending_maintenance_migration() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(11))
        .await
        .unwrap();

    let error = migrations::assert_exact_state(db.as_ref())
        .await
        .unwrap_err();

    assert!(error.to_string().contains("m0012_request_revisions"));
    assert!(relation_exists(db.as_ref(), "scope_request_change_blocks").await);
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.pending[0].name, "m0012_request_revisions");

    let worker_error = match connect_postgres_worker_store(target.schema_database_url()).await {
        Ok(_) => panic!("worker must refuse a pending maintenance migration"),
        Err(error) => error,
    };
    assert!(
        worker_error
            .to_string()
            .contains("does not match this binary")
    );
}

#[tokio::test]
async fn migration_inventory_orders_data_rewrite_and_contract_cutover_together() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(10))
        .await
        .unwrap();

    let plan = migrations::plan(db.as_ref()).await.unwrap();

    assert_eq!(plan.pending[0].name, "m0011_compact_request_started_events");
    assert_eq!(plan.pending[1].name, "m0012_request_revisions");
}

#[tokio::test]
async fn truthful_log_truncation_cutover_requires_maintenance() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(17))
        .await
        .unwrap();

    let plan = migrations::plan(db.as_ref()).await.unwrap();

    assert_eq!(plan.pending.len(), LATEST_MIGRATIONS.len() - 17);
    assert_eq!(plan.pending[0].name, "m0018_truthful_run_log_truncation");
    assert_eq!(plan.pending[1].name, "m0019_run_attempt_cache_observations");
    assert_eq!(plan.pending[2].name, "m0020_cloud_execution");
    assert_eq!(plan.pending[3].name, "m0021_cache_service_cutover");
    assert_eq!(plan.pending[4].name, "m0022_git_pack_spans");
    assert_eq!(plan.pending[5].name, "m0023_logical_run_sources");
    assert_eq!(plan.pending[6].name, "m0024_git_compaction_scheduler");
    assert_eq!(plan.pending[7].name, "m0025_visibility_change_sets");
    assert_eq!(plan.pending[8].name, "m0026_repository_landing_files");
    assert_eq!(plan.pending[9].name, "m0027_run_creation_sequence");
    assert_eq!(plan.pending[10].name, "m0028_repository_workflow_catalogs");
    assert_eq!(plan.pending[11].name, "m0029_exact_compatible_caches");
    assert_eq!(plan.pending[12].name, "m0030_cache_preparation_timings");
    assert_eq!(plan.pending[13].name, "m0031_provider_neutral_run_attempts");
    assert_eq!(plan.pending[14].name, "m0032_flat_discussion_replies");
    assert_eq!(plan.pending[15].name, "m0033_git_segment_streaming_v2");
    assert_eq!(plan.pending[16].name, "m0034_repository_incarnations");
    assert_eq!(plan.pending[17].name, "m0035_retired_git_storage_cutover");
    assert_eq!(plan.pending[18].name, "m0036_request_queue_indexes");
    assert_eq!(plan.pending[19].name, "m0037_repository_history_views");
    assert_eq!(plan.pending[20].name, "m0038_history_entry_positions");
    assert_eq!(plan.pending[21].name, "m0039_history_action_feed");
}

#[tokio::test]
async fn cache_service_cutover_requires_maintenance() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(20))
        .await
        .unwrap();

    let plan = migrations::plan(db.as_ref()).await.unwrap();

    assert_eq!(plan.pending.len(), LATEST_MIGRATIONS.len() - 20);
    assert_eq!(plan.pending[0].name, "m0021_cache_service_cutover");
    assert_eq!(plan.pending[1].name, "m0022_git_pack_spans");
    assert_eq!(plan.pending[2].name, "m0023_logical_run_sources");
    assert_eq!(plan.pending[3].name, "m0024_git_compaction_scheduler");
    assert_eq!(plan.pending[4].name, "m0025_visibility_change_sets");
    assert_eq!(plan.pending[5].name, "m0026_repository_landing_files");
    assert_eq!(plan.pending[6].name, "m0027_run_creation_sequence");
    assert_eq!(plan.pending[7].name, "m0028_repository_workflow_catalogs");
    assert_eq!(plan.pending[8].name, "m0029_exact_compatible_caches");
    assert_eq!(plan.pending[9].name, "m0030_cache_preparation_timings");
    assert_eq!(plan.pending[10].name, "m0031_provider_neutral_run_attempts");
    assert_eq!(plan.pending[11].name, "m0032_flat_discussion_replies");
    assert_eq!(plan.pending[12].name, "m0033_git_segment_streaming_v2");
    assert_eq!(plan.pending[13].name, "m0034_repository_incarnations");
    assert_eq!(plan.pending[14].name, "m0035_retired_git_storage_cutover");
    assert_eq!(plan.pending[15].name, "m0036_request_queue_indexes");
    assert_eq!(plan.pending[16].name, "m0037_repository_history_views");
    assert_eq!(plan.pending[17].name, "m0038_history_entry_positions");
    assert_eq!(plan.pending[18].name, "m0039_history_action_feed");
}

#[tokio::test]
async fn compaction_scheduler_requires_maintenance_before_startup() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(23))
        .await
        .unwrap();

    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.pending.len(), LATEST_MIGRATIONS.len() - 23);
    assert_eq!(plan.pending[0].name, "m0024_git_compaction_scheduler");
    assert_eq!(plan.pending[1].name, "m0025_visibility_change_sets");
    assert_eq!(plan.pending[2].name, "m0026_repository_landing_files");
    assert_eq!(plan.pending[3].name, "m0027_run_creation_sequence");
    assert_eq!(plan.pending[4].name, "m0028_repository_workflow_catalogs");
    assert_eq!(plan.pending[5].name, "m0029_exact_compatible_caches");
    assert_eq!(plan.pending[6].name, "m0030_cache_preparation_timings");
    assert_eq!(plan.pending[7].name, "m0031_provider_neutral_run_attempts");
    assert_eq!(plan.pending[8].name, "m0032_flat_discussion_replies");
    assert_eq!(plan.pending[9].name, "m0033_git_segment_streaming_v2");
    assert_eq!(plan.pending[10].name, "m0034_repository_incarnations");
    assert_eq!(plan.pending[11].name, "m0035_retired_git_storage_cutover");

    let error = migrations::assert_exact_state(db.as_ref())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("m0025_visibility_change_sets"));
    assert!(!relation_exists(db.as_ref(), "scope_git_compaction_jobs").await);
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), plan);
    assert_eq!(plan.pending[12].name, "m0036_request_queue_indexes");
    assert_eq!(plan.pending[13].name, "m0037_repository_history_views");
    assert_eq!(plan.pending[14].name, "m0038_history_entry_positions");
    assert_eq!(plan.pending[15].name, "m0039_history_action_feed");
}

#[tokio::test]
async fn request_queue_indexes_require_maintenance_before_startup() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(35))
        .await
        .unwrap();
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.pending[0].name, "m0036_request_queue_indexes");
    assert_eq!(plan.pending[1].name, "m0037_repository_history_views");
    assert_eq!(plan.pending[2].name, "m0038_history_entry_positions");
    assert_eq!(plan.pending[3].name, "m0039_history_action_feed");

    let error = match connect_postgres_store(target.schema_database_url()).await {
        Ok(_) => panic!("API startup must refuse the queue index build"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("m0036_request_queue_indexes"));
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), plan);

    let indexes = [
        "idx_scope_requests_open_queue",
        "idx_scope_requests_public_open_queue",
        "idx_scope_requests_draft_queue",
        "idx_scope_requests_closed_queue",
        "idx_scope_requests_public_closed_queue",
        "idx_scope_requests_public_search",
    ];
    for index in indexes {
        assert!(!relation_exists(db.as_ref(), index).await);
    }

    let writer = connect_writer_database(&target.schema_database_url())
        .await
        .unwrap();
    let error = apply_maintenance_migrations(target.schema_database_url(), Default::default())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("metadata writer"));
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), plan);
    writer.close().await.unwrap();

    apply_maintenance_migrations(target.schema_database_url(), Default::default())
        .await
        .unwrap();
    assert!(migrations::plan(db.as_ref()).await.unwrap().exact);
    for index in indexes {
        assert!(relation_exists(db.as_ref(), index).await);
    }
    let _store = connect_postgres_store(target.schema_database_url())
        .await
        .unwrap();
}

#[tokio::test]
async fn truthful_log_cutover_fences_protocol_six_runners() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(17))
        .await
        .unwrap();
    db.execute_unprepared(
        "INSERT INTO scope_users (id, handle, email, email_verified)
         VALUES ('user_v7', 'v7-owner', 'v7@scope.test', TRUE);
         INSERT INTO scope_runners (
             id, owner_user_id, secret_hash, version, protocol_version,
             capabilities, max_concurrent_jobs, enabled, created_at_unix,
             last_seen_at_unix
         ) VALUES (
             'runner_v6', 'user_v7', repeat('a', 64), '0.1.0', 6,
             '{\"operating_system\":\"linux\",\"architecture\":\"amd64\",\"container_engine\":\"docker\"}'::jsonb,
             1, TRUE, 1, NULL
         );",
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let cutover = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT state FROM scope_runner_protocol_cutover WHERE key = 'current'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "state")
        .unwrap();
    assert_eq!(cutover, "v7-open");
    let enabled = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT enabled FROM scope_runners WHERE id = 'runner_v6'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<bool>("", "enabled")
        .unwrap();
    assert!(!enabled);
    assert!(
        db.execute_unprepared("UPDATE scope_runners SET enabled = TRUE WHERE id = 'runner_v6'")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn maintenance_cutover_refuses_a_writer_after_its_pool_reconnects() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(11))
        .await
        .unwrap();
    let database_url = target.schema_database_url();
    let writer = connect_writer_database(&database_url).await.unwrap();

    let writer_pid = writer
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pg_backend_pid() AS pid".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "pid")
        .unwrap();
    db.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT pg_terminate_backend($1)",
        [writer_pid.into()],
    ))
    .await
    .unwrap();
    writer.ping().await.unwrap();

    let fence_error = verify_writer_fence_available(database_url.clone())
        .await
        .unwrap_err();
    assert!(fence_error.to_string().contains("writer still holds"));
    let migration_error = apply_maintenance_migrations(database_url.clone(), Default::default())
        .await
        .unwrap_err();
    assert!(migration_error.to_string().contains("writer still holds"));
    assert!(relation_exists(db.as_ref(), "scope_request_change_blocks").await);

    assert_eq!(
        terminate_metadata_writer_sessions(database_url.clone())
            .await
            .unwrap(),
        1
    );
    writer.close().await.unwrap();
    verify_writer_fence_available(database_url.clone())
        .await
        .unwrap();
    apply_maintenance_migrations(database_url, Default::default())
        .await
        .unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
    assert!(!relation_exists(db.as_ref(), "scope_request_change_blocks").await);
    assert!(relation_exists(db.as_ref(), "scope_request_revisions").await);

    let worker_store = connect_postgres_worker_store(target.schema_database_url())
        .await
        .unwrap();
    worker_store.admin().readiness_check().await.unwrap();

    db.execute_unprepared(
        "
            INSERT INTO seaql_migrations (version, applied_at)
            VALUES ('m9999_unknown', 0)
        ",
    )
    .await
    .unwrap();
    assert!(worker_store.admin().readiness_check().await.is_err());
}

#[tokio::test]
async fn runtime_startup_does_not_apply_even_additive_migrations() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(39))
        .await
        .unwrap();
    let before = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(before.pending[0].name, "m0040_repository_metadata");

    assert!(
        connect_postgres_store(target.schema_database_url())
            .await
            .is_err()
    );
    assert!(
        connect_postgres_worker_store(target.schema_database_url())
            .await
            .is_err()
    );
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), before);

    apply_maintenance_migrations(target.schema_database_url(), Default::default())
        .await
        .unwrap();
    let store = connect_postgres_store(target.schema_database_url())
        .await
        .unwrap();
    store.admin().readiness_check().await.unwrap();
}

#[tokio::test]
async fn migration_statement_timeout_rolls_back_schema_and_ledger() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(39))
        .await
        .unwrap();
    let before = migrations::plan(db.as_ref()).await.unwrap();
    db.execute_unprepared(
        "CREATE FUNCTION delay_migration_ledger() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN PERFORM pg_sleep(2); RETURN NEW; END $$;
         CREATE TRIGGER delay_migration_ledger BEFORE INSERT ON seaql_migrations
         FOR EACH ROW EXECUTE FUNCTION delay_migration_ledger();",
    )
    .await
    .unwrap();

    let error = migrations::apply_in_maintenance(
        db.as_ref(),
        migrations::MigrationLimits {
            lock_timeout_seconds: 1,
            statement_timeout_seconds: 1,
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("statement timeout"), "{error}");
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), before);
    assert!(
        db.execute_unprepared("SELECT description FROM scope_repositories")
            .await
            .is_err()
    );

    db.execute_unprepared("DROP TRIGGER delay_migration_ledger ON seaql_migrations")
        .await
        .unwrap();
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}

#[tokio::test]
async fn migration_lock_timeout_preserves_unapplied_inventory() {
    use sea_orm::TransactionTrait;
    let (_target, db, _lease) = isolated_database().await;
    let blocker = db.begin().await.unwrap();
    blocker.execute_unprepared(
        "SELECT pg_advisory_xact_lock(hashtextextended('scope:metadata-migrations:' || current_schema(), 0))"
    ).await.unwrap();
    let error = migrations::apply_in_maintenance(
        db.as_ref(),
        migrations::MigrationLimits {
            lock_timeout_seconds: 1,
            statement_timeout_seconds: 10,
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("lock timeout"), "{error}");
    assert!(!relation_exists(db.as_ref(), "seaql_migrations").await);
    blocker.rollback().await.unwrap();
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
