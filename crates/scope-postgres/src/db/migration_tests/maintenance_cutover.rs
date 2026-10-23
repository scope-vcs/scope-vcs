use super::*;
use crate::db::{
    apply_maintenance_migrations, connect_postgres_store, connect_writer_database,
    terminate_metadata_writer_sessions, verify_writer_fence_available,
};

#[tokio::test]
async fn ordinary_startup_refuses_pending_maintenance_migration() {
    let (target, db, _lease) = isolated_database().await;

    let error = migrations::assert_exact_state(db.as_ref())
        .await
        .unwrap_err();

    assert!(error.to_string().contains("have not been applied"));
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(
        plan.pending[0].name,
        migrations::Migrator::migrations()[0].name()
    );
    assert!(plan.applied.is_empty());

    let api_error = match connect_postgres_store(
        target.schema_database_url(),
        crate::db::connection::DEFAULT_METADATA_POOL_MAX_CONNECTIONS,
    )
    .await
    {
        Ok(_) => panic!("API must refuse a pending maintenance migration"),
        Err(error) => error,
    };
    assert!(api_error.to_string().contains("have not been applied"));
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), plan);
}

#[tokio::test]
async fn maintenance_cutover_refuses_a_writer_after_its_pool_reconnects() {
    let (target, db, _lease) = isolated_database().await;
    let database_url = target.schema_database_url();
    let writer = connect_writer_database(
        &database_url,
        database_url.parse().unwrap(),
        crate::db::connection::DEFAULT_METADATA_POOL_MAX_CONNECTIONS,
    )
    .await
    .unwrap();

    let writer_pid = writer
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pg_backend_pid() AS pid".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "pid")
        .unwrap();
    db.execute_raw(Statement::from_sql_and_values(
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
    assert!(!migrations::plan(db.as_ref()).await.unwrap().exact);

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
    assert!(migrations::plan(db.as_ref()).await.unwrap().exact);

    let worker_store = connect_postgres_store(
        target.schema_database_url(),
        crate::db::connection::DEFAULT_METADATA_POOL_MAX_CONNECTIONS,
    )
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
async fn migration_statement_timeout_rolls_back_schema_and_ledger() {
    use sea_orm::TransactionTrait;

    let (_target, db, _lease) = isolated_database().await;
    let before = migrations::plan(db.as_ref()).await.unwrap();
    let blocker = db.begin().await.unwrap();
    blocker
        .execute_unprepared(
            "SELECT pg_advisory_xact_lock(hashtextextended('scope:metadata-extension:pg_trgm', 0))",
        )
        .await
        .unwrap();

    let error = migrations::apply_in_maintenance(
        db.as_ref(),
        migrations::MigrationLimits {
            lock_timeout_seconds: 10,
            statement_timeout_seconds: 1,
        },
    )
    .await
    .unwrap_err();
    blocker.rollback().await.unwrap();

    assert!(error.to_string().contains("statement timeout"), "{error}");
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), before);
    assert!(!relation_exists(db.as_ref(), "seaql_migrations").await);
    assert!(!relation_exists(db.as_ref(), "scope_users").await);
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
    blocker
        .execute_unprepared(
            "SELECT pg_advisory_xact_lock(hashtextextended(
                'scope:metadata-migrations:' || current_schema(), 0
            ))",
        )
        .await
        .unwrap();
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

#[tokio::test]
async fn preflight_requires_empty_schema_for_an_empty_ledger_without_persisting_metadata() {
    let (_target, db, _lease) = isolated_database().await;
    let before = migrations::plan(db.as_ref()).await.unwrap();
    assert!(before.applied.is_empty());
    assert_eq!(
        migrations::preflight(&db, Default::default())
            .await
            .unwrap(),
        before
    );
    assert!(!relation_exists(&db, "seaql_migrations").await);
    assert!(!relation_exists(&db, "scope_users").await);
    assert!(!relation_exists(&db, "pg_temp.scope_baseline_expression_inventory").await);

    db.execute_unprepared(
        "CREATE TABLE untracked (id integer CHECK (id > 0)); INSERT INTO untracked VALUES (7)",
    )
    .await
    .unwrap();
    let error = migrations::preflight(&db, Default::default())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("requires an empty schema"),
        "{error}"
    );
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), before);
    assert!(!relation_exists(&db, "seaql_migrations").await);
    assert!(!relation_exists(&db, "scope_users").await);
    assert!(!relation_exists(&db, "pg_temp.scope_baseline_expression_inventory").await);
    let retained = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id FROM untracked",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i32>("", "id")
        .unwrap();
    assert_eq!(retained, 7);
}
