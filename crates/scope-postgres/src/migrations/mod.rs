mod baseline;
mod m0043_retire_git_manifests;

use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, Statement, TransactionTrait,
};
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use serde::Serialize;

const MIGRATION_LOCK: &str = "scope:metadata-migrations";
const MIGRATION_TABLE: &str = "seaql_migrations";

pub struct Migrator;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PendingMigration {
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MigrationPlan {
    pub exact: bool,
    pub applied: Vec<String>,
    pub pending: Vec<PendingMigration>,
}

#[sea_orm_migration::async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(baseline::Migration),
            Box::new(m0043_retire_git_manifests::Migration),
        ]
    }
}

/// Per-statement limits for the migration transaction, independent of outage reporting.
#[derive(Clone, Copy, Debug)]
pub struct MigrationLimits {
    pub lock_timeout_seconds: u32,
    pub statement_timeout_seconds: u32,
}

impl Default for MigrationLimits {
    fn default() -> Self {
        Self {
            lock_timeout_seconds: 120,
            statement_timeout_seconds: 3600,
        }
    }
}

pub async fn apply_in_maintenance(
    db: &DatabaseConnection,
    limits: MigrationLimits,
) -> Result<(), DbErr> {
    let tx = db.begin().await?;
    set_operation_limits(&tx, limits).await?;
    lock_migration_inventory(&tx).await?;
    plan(&tx).await?;
    if baseline::is_original_chain(&applied_migration_names(&tx).await?) {
        baseline::bridge(&tx).await?;
    }
    Migrator::up(&tx, None).await?;
    assert_exact_state(&tx).await?;
    tx.commit().await
}

/// Inspect baseline bridge eligibility while writers remain online. Comparison
/// metadata is transactional; neither the migration ledger nor user data changes.
pub async fn preflight(
    db: &DatabaseConnection,
    limits: MigrationLimits,
) -> Result<MigrationPlan, DbErr> {
    let tx = db.begin().await?;
    let result = async {
        set_operation_limits(&tx, limits).await?;
        lock_migration_inventory(&tx).await?;
        let plan = plan(&tx).await?;
        if plan.applied.is_empty() {
            baseline::assert_empty_schema(&tx).await?;
        } else if baseline::is_original_chain(&plan.applied) || plan.applied == [baseline::NAME] {
            baseline::assert_baseline_schema(&tx).await?;
        }
        Ok(plan)
    }
    .await;
    // Explicit rollback also cleans up failed comparisons before returning.
    tx.rollback().await?;
    result
}

async fn set_operation_limits<C: ConnectionTrait>(
    db: &C,
    limits: MigrationLimits,
) -> Result<(), DbErr> {
    if limits.lock_timeout_seconds == 0 || limits.statement_timeout_seconds == 0 {
        return Err(DbErr::Custom(
            "migration operation limits must be positive".to_string(),
        ));
    }
    db.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT set_config('lock_timeout', $1, true), set_config('statement_timeout', $2, true)",
        [
            format!("{}s", limits.lock_timeout_seconds).into(),
            format!("{}s", limits.statement_timeout_seconds).into(),
        ],
    ))
    .await?;
    Ok(())
}

pub async fn plan<C>(db: &C) -> Result<MigrationPlan, DbErr>
where
    C: ConnectionTrait,
{
    let actual = applied_migration_names(db).await?;
    let migrations = Migrator::migrations();
    let expected = migrations
        .iter()
        .map(|migration| migration.name().to_string())
        .collect::<Vec<_>>();
    let original_chain = baseline::is_original_chain(&actual);
    if !original_chain && !expected.starts_with(&actual) {
        return Err(DbErr::Custom(format!(
            "Scope metadata migration ledger is not a canonical prefix: expected [{}], found [{}]. The baseline bridge accepts only the exact original chain through m0042_request_media. Older retained databases must first run original-chain revision {}",
            expected.join(", "),
            actual.join(", "),
            baseline::ORIGINAL_CHAIN_REVISION
        )));
    }
    let pending = migrations
        .into_iter()
        .skip(if original_chain { 0 } else { actual.len() })
        .map(|migration| PendingMigration {
            name: migration.name().to_string(),
        })
        .collect::<Vec<_>>();
    Ok(MigrationPlan {
        exact: pending.is_empty(),
        applied: actual,
        pending,
    })
}

async fn lock_migration_inventory<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    db.execute_unprepared(&format!(
        "SELECT pg_advisory_xact_lock(
            hashtextextended('{MIGRATION_LOCK}:' || current_schema(), 0)
        )"
    ))
    .await?;
    Ok(())
}

pub async fn assert_exact_state<C>(db: &C) -> Result<(), DbErr>
where
    C: ConnectionTrait,
{
    let table_exists = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT to_regclass(
                format('%I.%I', current_schema(), $1)
            ) IS NOT NULL AS exists",
            [MIGRATION_TABLE.into()],
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("PostgreSQL did not report migration state".to_string()))?
        .try_get::<bool>("", "exists")?;
    if !table_exists {
        return Err(DbErr::Custom(
            "Scope metadata migrations have not been applied".to_string(),
        ));
    }

    let actual = applied_migration_names(db).await?;
    let expected = migration_names();
    if actual != expected {
        return Err(DbErr::Custom(format!(
            "Scope metadata migration state does not match this binary: expected [{}], found [{}]",
            expected.join(", "),
            actual.join(", ")
        )));
    }
    Ok(())
}

async fn applied_migration_names<C>(db: &C) -> Result<Vec<String>, DbErr>
where
    C: ConnectionTrait,
{
    let table_exists = db
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT to_regclass(
                format('%I.%I', current_schema(), $1)
            ) IS NOT NULL AS exists",
            [MIGRATION_TABLE.into()],
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("PostgreSQL did not report migration state".to_string()))?
        .try_get::<bool>("", "exists")?;
    if !table_exists {
        return Ok(Vec::new());
    }

    db.query_all(Statement::from_string(
        DatabaseBackend::Postgres,
        format!("SELECT version FROM {MIGRATION_TABLE} ORDER BY version"),
    ))
    .await?
    .into_iter()
    .map(|row| row.try_get::<String>("", "version"))
    .collect()
}

fn migration_names() -> Vec<String> {
    let mut names = Migrator::migrations()
        .into_iter()
        .map(|migration| migration.name().to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}
