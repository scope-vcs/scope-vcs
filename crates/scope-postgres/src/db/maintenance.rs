//! Out-of-band schema maintenance entry points.

use super::{AdminStore, ExclusiveWriterFence, workflow_catalogs};
use crate::error::PostgresError;
use crate::migrations::{MigrationLimits, MigrationPlan};
use sea_orm::{ConnectionTrait, Database};

impl AdminStore {
    pub async fn readiness_check(&self) -> Result<(), PostgresError> {
        crate::migrations::assert_exact_state(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)
    }
}

pub async fn migration_preflight(
    database_url: String,
    limits: MigrationLimits,
) -> anyhow::Result<MigrationPlan> {
    let db = Database::connect(database_url).await?;
    Ok(crate::migrations::preflight(&db, limits).await?)
}

pub async fn migration_plan(database_url: String) -> anyhow::Result<MigrationPlan> {
    let db = Database::connect(database_url).await?;
    Ok(crate::migrations::plan(&db).await?)
}

pub async fn repository_workflow_catalogs_for_maintenance(
    database_url: String,
) -> anyhow::Result<Vec<scope_domain::runs::catalog::RepositoryWorkflowCatalog>> {
    let db = Database::connect(database_url).await?;
    crate::migrations::plan(&db).await?;
    let exists = db
        .query_one(sea_orm::Statement::from_string(
            db.get_database_backend(),
            "SELECT to_regclass(format('%I.scope_repository_workflow_catalogs', current_schema())) IS NOT NULL AS exists",
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("PostgreSQL did not report workflow catalog schema state"))?
        .try_get::<bool>("", "exists")?;
    if !exists {
        return Ok(Vec::new());
    }
    Ok(workflow_catalogs::load_repository_workflow_catalogs(&db).await?)
}

pub async fn verify_schema(database_url: String) -> anyhow::Result<()> {
    let db = Database::connect(database_url).await?;
    crate::migrations::assert_exact_state(&db).await?;
    Ok(())
}

pub async fn apply_maintenance_migrations(
    database_url: String,
    limits: MigrationLimits,
) -> anyhow::Result<()> {
    let fence = ExclusiveWriterFence::acquire(&database_url).await?;
    let db = Database::connect(database_url).await?;
    let migration_result = crate::migrations::apply_in_maintenance(&db, limits).await;
    let release_result = fence.release().await;
    migration_result?;
    release_result?;
    Ok(())
}
