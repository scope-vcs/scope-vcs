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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MigrationImpact {
    Online,
    MaintenanceRequired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PendingMigration {
    pub name: String,
    pub impact: MigrationImpact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MigrationPlan {
    pub exact: bool,
    pub pending: Vec<PendingMigration>,
}

struct MigrationSpec {
    migration: Box<dyn MigrationTrait>,
    impact: MigrationImpact,
}

fn spec(migration: impl MigrationTrait + 'static, impact: MigrationImpact) -> MigrationSpec {
    MigrationSpec {
        migration: Box::new(migration),
        impact,
    }
}

fn inventory() -> Vec<MigrationSpec> {
    use MigrationImpact::MaintenanceRequired;
    vec![
        spec(baseline::Migration, MaintenanceRequired),
        spec(m0043_retire_git_manifests::Migration, MaintenanceRequired),
    ]
}

#[sea_orm_migration::async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        inventory().into_iter().map(|spec| spec.migration).collect()
    }
}

pub async fn apply_in_maintenance(db: &DatabaseConnection) -> Result<(), DbErr> {
    let tx = db.begin().await?;
    lock_migration_inventory(&tx).await?;
    plan(&tx).await?;
    if baseline::is_original_chain(&applied_migration_names(&tx).await?) {
        baseline::bridge(&tx).await?;
    }
    Migrator::up(&tx, None).await?;
    assert_exact_state(&tx).await?;
    tx.commit().await
}

pub async fn apply_online(db: &DatabaseConnection) -> Result<(), DbErr> {
    let tx = db.begin().await?;
    lock_migration_inventory(&tx).await?;
    let pending = plan(&tx).await?.pending;
    let online_count = pending
        .iter()
        .take_while(|migration| migration.impact == MigrationImpact::Online)
        .count();
    if online_count > 0 {
        Migrator::up(&tx, Some(online_count as u32)).await?;
    }
    tx.commit().await?;

    let remaining = plan(db).await?.pending;
    if let Some(blocked) = remaining.first() {
        return Err(DbErr::Custom(format!(
            "migration {} requires a maintenance cutover; ordinary startup will not apply it",
            blocked.name
        )));
    }
    Ok(())
}

pub async fn plan<C>(db: &C) -> Result<MigrationPlan, DbErr>
where
    C: ConnectionTrait,
{
    let actual = applied_migration_names(db).await?;
    let specs = inventory();
    let expected = specs
        .iter()
        .map(|spec| spec.migration.name().to_string())
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
    let pending = specs
        .into_iter()
        .skip(if original_chain { 0 } else { actual.len() })
        .map(|spec| PendingMigration {
            name: spec.migration.name().to_string(),
            impact: spec.impact,
        })
        .collect::<Vec<_>>();
    Ok(MigrationPlan {
        exact: pending.is_empty(),
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
