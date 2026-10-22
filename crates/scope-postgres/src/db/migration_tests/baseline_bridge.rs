use super::current_schema_baseline::{baseline_database, sequence_state};
use super::*;
use crate::db::connect_postgres_store;

async fn retained_bridged_database(db: &DatabaseConnection) -> Vec<&'static str> {
    baseline_database(db).await;
    let bridged = BRIDGED_LEDGER.lines().collect::<Vec<_>>();
    replace_ledger(db, &bridged).await;
    bridged
}

#[tokio::test]
async fn the_bridge_replaces_the_previous_ledger_after_verifying_the_schema() {
    let (target, db, _lease) = isolated_database().await;
    let bridged = retained_bridged_database(&db).await;
    let before = representative_business_snapshot(&db).await;
    let sequence = sequence_state(&db).await;

    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.applied, bridged);
    assert_eq!(
        plan.pending
            .iter()
            .map(|migration| migration.name.as_str())
            .collect::<Vec<_>>(),
        migrations::Migrator::migrations()
            .iter()
            .map(|migration| migration.name())
            .collect::<Vec<_>>()
    );
    assert!(!plan.exact);
    assert!(plan.metadata_restore_safe);
    let startup_error = migrations::assert_exact_state(db.as_ref())
        .await
        .unwrap_err();
    assert!(
        startup_error
            .to_string()
            .contains("does not match this binary"),
        "{startup_error}"
    );
    assert!(
        connect_postgres_store(
            target.schema_database_url(),
            crate::db::connection::MetadataPoolLimits::default(),
        )
        .await
        .is_err()
    );
    assert_eq!(
        migrations::preflight(&db, Default::default())
            .await
            .unwrap(),
        plan
    );
    assert_eq!(applied_versions(&db).await, bridged);

    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();

    migrations::assert_exact_state(db.as_ref()).await.unwrap();
    assert_eq!(representative_business_snapshot(&db).await, before);
    assert_eq!(sequence_state(&db).await, sequence);
    let completed = migrations::plan(db.as_ref()).await.unwrap();
    assert!(completed.exact);
    assert_eq!(
        migrations::preflight(&db, Default::default())
            .await
            .unwrap(),
        completed
    );
    connect_postgres_store(
        target.schema_database_url(),
        crate::db::connection::MetadataPoolLimits::default(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_bridge_refuses_schema_drift_without_changing_the_ledger() {
    let (_target, db, _lease) = isolated_database().await;
    let bridged = retained_bridged_database(&db).await;
    let drift = "ALTER TABLE scope_request_check_evaluations DROP COLUMN changes_github_workflows";
    db.execute_unprepared(drift).await.unwrap();
    let before = representative_business_snapshot(&db).await;

    for error in [
        migrations::preflight(&db, Default::default())
            .await
            .unwrap_err(),
        migrations::apply_in_maintenance(db.as_ref(), Default::default())
            .await
            .unwrap_err(),
    ] {
        assert!(
            error.to_string().contains("schema drift"),
            "{drift}: {error}"
        );
    }
    assert_eq!(applied_versions(&db).await, bridged);
    assert_eq!(representative_business_snapshot(&db).await, before);
}
