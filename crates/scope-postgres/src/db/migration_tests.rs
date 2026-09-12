use super::{
    TestDatabaseTarget,
    test_support::{TestSchemaLease, connect_isolated_test_database},
};
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use sea_orm_migration::MigratorTrait;
use std::sync::Arc;

mod cache_schema;
mod current_schema_baseline;
mod fresh_schema;
mod git_manifest_retirement;
mod git_segment_schema;
mod maintenance_cutover;
mod repository_landing_files;
mod repository_workflow_catalogs;

const LATEST_MIGRATIONS: &[&str] = &[
    "m0042_current_schema_baseline",
    "m0043_retire_git_manifests",
    "m0044_request_attention",
    "m0045_dependency_analysis",
];

pub(super) async fn isolated_database() -> (
    TestDatabaseTarget,
    Arc<DatabaseConnection>,
    Arc<TestSchemaLease>,
) {
    let target = TestDatabaseTarget::required().unwrap();
    let (db, lease) = connect_isolated_test_database(&target).await.unwrap();
    (target, db, lease)
}

pub(super) async fn relation_exists(db: &DatabaseConnection, relation: &str) -> bool {
    db.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT to_regclass($1) IS NOT NULL AS exists",
        [relation.into()],
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<bool>("", "exists")
    .unwrap()
}

pub(super) async fn applied_versions(db: &DatabaseConnection) -> Vec<String> {
    db.query_all(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT version FROM seaql_migrations ORDER BY version".to_string(),
    ))
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.try_get::<String>("", "version").unwrap())
    .collect()
}

async fn representative_business_snapshot(db: &DatabaseConnection) -> String {
    db.query_one(Statement::from_string(
        DatabaseBackend::Postgres,
        "
            SELECT jsonb_build_object(
                'users', (SELECT jsonb_agg(to_jsonb(item) ORDER BY id) FROM scope_users item),
                'auth', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY provider, subject)
                    FROM scope_auth_identities item
                ),
                'repositories', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY id)
                    FROM scope_repositories item
                ),
                'requests', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY id)
                    FROM scope_requests item
                ),
                'workflow_revisions', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY digest)
                    FROM scope_workflow_revisions item
                ),
                'runs', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY id)
                    FROM scope_runs item
                ),
                'outbox', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY id)
                    FROM scope_outbox_jobs item
                ),
                'projections', (
                    SELECT jsonb_agg(to_jsonb(item) ORDER BY repo_id, source, audience)
                    FROM scope_projection_read_models item
                )
            )::text AS value
        "
        .to_string(),
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get::<String>("", "value")
    .unwrap()
}

#[tokio::test]
async fn reapplying_latest_migrations_is_a_data_preserving_noop() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    db.execute_unprepared(
        "
            INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('user_latest', 'latest', 'latest@scope.test', TRUE)
        ",
    )
    .await
    .unwrap();
    let before = representative_business_snapshot(db.as_ref()).await;

    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();

    assert_eq!(representative_business_snapshot(db.as_ref()).await, before);
    assert_eq!(applied_versions(db.as_ref()).await, LATEST_MIGRATIONS);
}

#[tokio::test]
async fn concurrent_maintenance_migration_attempts_serialize() {
    let (_target, db, _lease) = isolated_database().await;

    let (first, second) = tokio::join!(
        migrations::apply_in_maintenance(db.as_ref(), Default::default()),
        migrations::apply_in_maintenance(db.as_ref(), Default::default())
    );

    first.unwrap();
    second.unwrap();
    assert_eq!(applied_versions(db.as_ref()).await, LATEST_MIGRATIONS);
}

#[tokio::test]
async fn exact_state_check_is_read_only_and_rejects_behind_and_ahead() {
    let (_target, db, _lease) = isolated_database().await;

    assert!(migrations::assert_exact_state(db.as_ref()).await.is_err());
    assert!(!relation_exists(db.as_ref(), "seaql_migrations").await);

    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    db.execute_unprepared(
        "DELETE FROM seaql_migrations WHERE version = 'm0042_current_schema_baseline'",
    )
    .await
    .unwrap();
    assert!(migrations::assert_exact_state(db.as_ref()).await.is_err());

    db.execute_unprepared(
        "
            INSERT INTO seaql_migrations (version, applied_at)
            VALUES ('m0042_current_schema_baseline', 0), ('m9999_unknown', 0)
        ",
    )
    .await
    .unwrap();
    assert!(migrations::assert_exact_state(db.as_ref()).await.is_err());
}
