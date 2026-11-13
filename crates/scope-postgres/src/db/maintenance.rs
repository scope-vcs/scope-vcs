use super::{AdminStore, ExclusiveWriterFence};
use crate::error::PostgresError;
use crate::migrations::{MigrationLimits, MigrationPlan};
use sea_orm::Database;

impl AdminStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "readiness_check"))]
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
