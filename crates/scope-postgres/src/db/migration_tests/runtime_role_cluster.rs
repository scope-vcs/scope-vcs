use super::*;

#[tokio::test]
async fn runtime_role_cluster_receives_the_production_migrations() {
    let Ok(url) = std::env::var("SCOPE_RUNTIME_ROLE_MIGRATION_URL") else {
        return;
    };
    let db = sea_orm::Database::connect(url).await.unwrap();
    migrations::apply_in_maintenance(&db, migrations::MigrationLimits::default())
        .await
        .unwrap();
}
