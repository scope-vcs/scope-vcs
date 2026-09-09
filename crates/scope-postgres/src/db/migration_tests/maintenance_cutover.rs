use super::*;
use crate::db::{
    apply_maintenance_migrations, connect_postgres_store, connect_postgres_worker_store,
    connect_writer_database, terminate_metadata_writer_sessions, verify_writer_fence_available,
};

#[tokio::test]
async fn ordinary_startup_refuses_pending_maintenance_migration() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(1))
        .await
        .unwrap();

    let error = migrations::assert_exact_state(db.as_ref())
        .await
        .unwrap_err();

    assert!(error.to_string().contains("m0043_retire_git_manifests"));
    assert_eq!(applied_versions(&db).await.len(), 1);
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.pending[0].name, "m0043_retire_git_manifests");
    assert_eq!(plan.applied, ["m0042_current_schema_baseline"]);

    let api_error = match connect_postgres_store(target.schema_database_url()).await {
        Ok(_) => panic!("API must refuse a pending maintenance migration"),
        Err(error) => error,
    };
    assert!(api_error.to_string().contains("does not match this binary"));
    assert_eq!(migrations::plan(db.as_ref()).await.unwrap(), plan);

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
async fn maintenance_cutover_refuses_a_writer_after_its_pool_reconnects() {
    let (target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(1))
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
async fn migration_statement_timeout_rolls_back_schema_and_ledger() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(1))
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
        db.execute_unprepared("SELECT manifest_object_key FROM scope_git_heads")
            .await
            .is_ok()
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
