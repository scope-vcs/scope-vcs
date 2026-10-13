use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, Statement};

use super::baseline;

pub(super) const ORIGINAL_CHAIN_REVISION: &str = "26619ef4bb5ca7fe8bb938ac3f5ca0db73c41282";
const BRIDGED_LEDGER: &str = include_str!("m0084_bridged_ledger.txt");

pub(super) fn is_bridged_chain(actual: &[String]) -> bool {
    actual.iter().map(String::as_str).eq(BRIDGED_LEDGER.lines())
}

pub(super) fn unsupported_ledger_hint() -> String {
    format!(
        "The baseline bridge accepts only the exact ledger m0042_current_schema_baseline through m0083_request_workflow_changes; a database behind it must first reach that ledger with original-chain revision {ORIGINAL_CHAIN_REVISION}"
    )
}

pub(super) async fn bridge<C: ConnectionTrait>(db: &C) -> Result<(), DbErr> {
    db.execute_unprepared("LOCK TABLE seaql_migrations IN ACCESS EXCLUSIVE MODE")
        .await?;
    if !is_bridged_chain(&super::applied_migration_names(db).await?) {
        return Err(DbErr::Custom(format!(
            "the baseline bridge requires the exact bridged ledger. {}",
            unsupported_ledger_hint()
        )));
    }
    baseline::assert_baseline_schema(db).await?;
    db.execute_unprepared("DELETE FROM seaql_migrations")
        .await?;
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO seaql_migrations (version, applied_at) VALUES ($1, EXTRACT(EPOCH FROM now())::bigint)",
        [baseline::NAME.into()],
    ))
    .await?;
    Ok(())
}
