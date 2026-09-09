mod m0001_adopt_v6;
mod m0002_retire_reset_schema;
mod m0003_structured_run_attempts;
mod m0004_runner_protocol_cutover;
mod m0005_projection_head_oid;
mod m0006_drop_request_credits;
mod m0007_drop_review_ceremony;
mod m0008_one_way_request_submission;
mod m0009_request_ratings;
mod m0010_file_visibility_source_of_truth;
mod m0011_compact_request_started_events;
mod m0012_request_revisions;
mod m0013_workflow_jobs;
mod m0014_run_jobs;
mod m0015_runner_capacity;
mod m0016_workflow_runtime_contract;
mod m0017_run_history_indexes;
mod m0018_truthful_run_log_truncation;
mod m0019_run_attempt_cache_observations;
mod m0020_cloud_execution;
mod m0021_cache_service_cutover;
mod m0022_git_pack_spans;
mod m0023_logical_run_sources;
mod m0024_git_compaction_scheduler;
mod m0025_visibility_change_sets;
mod m0026_repository_landing_files;
mod m0027_run_creation_sequence;
mod m0028_repository_workflow_catalogs;
mod m0029_exact_compatible_caches;
mod m0030_cache_preparation_timings;
mod m0031_provider_neutral_run_attempts;
mod m0032_flat_discussion_replies;
mod m0033_git_segment_streaming_v2;
mod m0034_repository_incarnations;
mod m0035_retired_git_storage_cutover;
mod m0036_request_queue_indexes;
mod m0037_repository_history_views;
mod m0038_history_entry_positions;
mod m0039_history_action_feed;
mod m0040_repository_metadata;
mod m0041_history_occurrence_time;
mod m0042_request_media;

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
            Box::new(m0001_adopt_v6::Migration),
            Box::new(m0002_retire_reset_schema::Migration),
            Box::new(m0003_structured_run_attempts::Migration),
            Box::new(m0004_runner_protocol_cutover::Migration),
            Box::new(m0005_projection_head_oid::Migration),
            Box::new(m0006_drop_request_credits::Migration),
            Box::new(m0007_drop_review_ceremony::Migration),
            Box::new(m0008_one_way_request_submission::Migration),
            Box::new(m0009_request_ratings::Migration),
            Box::new(m0010_file_visibility_source_of_truth::Migration),
            Box::new(m0011_compact_request_started_events::Migration),
            Box::new(m0012_request_revisions::Migration),
            Box::new(m0013_workflow_jobs::Migration),
            Box::new(m0014_run_jobs::Migration),
            Box::new(m0015_runner_capacity::Migration),
            Box::new(m0016_workflow_runtime_contract::Migration),
            Box::new(m0017_run_history_indexes::Migration),
            Box::new(m0018_truthful_run_log_truncation::Migration),
            Box::new(m0019_run_attempt_cache_observations::Migration),
            Box::new(m0020_cloud_execution::Migration),
            Box::new(m0021_cache_service_cutover::Migration),
            Box::new(m0022_git_pack_spans::Migration),
            Box::new(m0023_logical_run_sources::Migration),
            Box::new(m0024_git_compaction_scheduler::Migration),
            Box::new(m0025_visibility_change_sets::Migration),
            Box::new(m0026_repository_landing_files::Migration),
            Box::new(m0027_run_creation_sequence::Migration),
            Box::new(m0028_repository_workflow_catalogs::Migration),
            Box::new(m0029_exact_compatible_caches::Migration),
            Box::new(m0030_cache_preparation_timings::Migration),
            Box::new(m0031_provider_neutral_run_attempts::Migration),
            Box::new(m0032_flat_discussion_replies::Migration),
            Box::new(m0033_git_segment_streaming_v2::Migration),
            Box::new(m0034_repository_incarnations::Migration),
            Box::new(m0035_retired_git_storage_cutover::Migration),
            Box::new(m0036_request_queue_indexes::Migration),
            Box::new(m0037_repository_history_views::Migration),
            Box::new(m0038_history_entry_positions::Migration),
            Box::new(m0039_history_action_feed::Migration),
            Box::new(m0040_repository_metadata::Migration),
            Box::new(m0041_history_occurrence_time::Migration),
            Box::new(m0042_request_media::Migration),
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
    if limits.lock_timeout_seconds == 0 || limits.statement_timeout_seconds == 0 {
        return Err(DbErr::Custom(
            "migration operation limits must be positive".to_string(),
        ));
    }
    let tx = db.begin().await?;
    tx.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT set_config('lock_timeout', $1, true), set_config('statement_timeout', $2, true)",
        [
            format!("{}s", limits.lock_timeout_seconds).into(),
            format!("{}s", limits.statement_timeout_seconds).into(),
        ],
    ))
    .await?;
    lock_migration_inventory(&tx).await?;
    Migrator::up(&tx, None).await?;
    assert_exact_state(&tx).await?;
    tx.commit().await
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
    if !expected.starts_with(&actual) {
        return Err(DbErr::Custom(format!(
            "Scope metadata migration ledger is not a canonical prefix: expected [{}], found [{}]",
            expected.join(", "),
            actual.join(", ")
        )));
    }
    let pending = migrations
        .into_iter()
        .skip(actual.len())
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
