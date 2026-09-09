use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, Statement};
use sea_orm_migration::{MigrationName, MigrationTrait, SchemaManager};
use serde_json::Value;

pub(super) const NAME: &str = "m0042_current_schema_baseline";
pub(super) const ORIGINAL_CHAIN_REVISION: &str = "578bec00da088598919082b35a7153f62bf0b860";
const SCHEMA: &str = include_str!("current_schema.sql");

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        NAME
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let state = schema_inventory(db).await?;
        if state.as_object().is_none_or(|state| {
            state.values().any(|objects| match objects {
                Value::Array(objects) => !objects.is_empty(),
                Value::Object(objects) => !objects.is_empty(),
                _ => true,
            })
        }) {
            return Err(DbErr::Custom(
                "the current-schema baseline requires an empty schema; restore retained data with the original-chain maintenance binary before bridging".into(),
            ));
        }
        db.execute_unprepared(SCHEMA).await?;
        Ok(())
    }
}

pub(super) fn is_original_chain(actual: &[String]) -> bool {
    actual
        .iter()
        .map(String::as_str)
        .eq(include_str!("baseline_ledger.txt").lines())
}

/// Called only inside the maintenance transaction with the migration lock held.
pub(super) async fn bridge<C: ConnectionTrait>(db: &C) -> Result<(), DbErr> {
    db.execute_unprepared("LOCK TABLE seaql_migrations IN ACCESS EXCLUSIVE MODE")
        .await?;
    if !is_original_chain(&super::applied_migration_names(db).await?) {
        return Err(DbErr::Custom("the baseline bridge requires the exact original migration ledger through m0042_request_media".into()));
    }
    assert_baseline_schema(db).await?;
    db.execute_unprepared("DELETE FROM seaql_migrations")
        .await?;
    db.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO seaql_migrations (version, applied_at) VALUES ($1, EXTRACT(EPOCH FROM now())::bigint)",
        [NAME.into()],
    ))
    .await?;
    Ok(())
}

/// Build the expected schema on this server so catalog formatting is independent
/// of PostgreSQL versions. Only metadata is created in the comparison schema.
pub(super) async fn assert_baseline_schema<C: ConnectionTrait>(db: &C) -> Result<(), DbErr> {
    let actual = schema_inventory(db).await?;
    let state = db.query_one(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT current_setting('search_path') AS search_path,
                'scope_baseline_check_' || pg_backend_pid() || '_' || txid_current() AS comparison_schema",
    )).await?.ok_or_else(|| DbErr::Custom("PostgreSQL did not report schema context".into()))?;
    let search_path = state.try_get::<String>("", "search_path")?;
    let comparison_schema = state.try_get::<String>("", "comparison_schema")?;
    db.execute_unprepared(&format!("CREATE SCHEMA {comparison_schema}"))
        .await?;
    set_search_path(db, &comparison_schema).await?;
    db.execute_unprepared(SCHEMA).await?;
    let expected = schema_inventory(db).await?;
    set_search_path(db, &search_path).await?;
    db.execute_unprepared(&format!("DROP SCHEMA {comparison_schema} CASCADE"))
        .await?;
    if actual != expected {
        let differences = expected
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| actual.get(*key) != expected.get(*key))
            .cloned()
            .collect::<Vec<_>>();
        return Err(DbErr::Custom(format!(
            "Scope baseline bridge refused schema drift in {}; restore and verify revision {ORIGINAL_CHAIN_REVISION} before retrying",
            differences.join(", ")
        )));
    }
    Ok(())
}

async fn set_search_path<C: ConnectionTrait>(db: &C, search_path: &str) -> Result<(), DbErr> {
    db.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT set_config('search_path', $1, true)",
        [search_path.into()],
    ))
    .await?;
    Ok(())
}

pub(super) async fn schema_inventory<C: ConnectionTrait>(db: &C) -> Result<Value, DbErr> {
    let schema = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT current_schema() AS name",
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("PostgreSQL did not report current schema".into()))?
        .try_get::<String>("", "name")?;
    let mut inventory = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            include_str!("schema_inventory.sql"),
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("PostgreSQL did not report schema inventory".into()))?
        .try_get::<Value>("", "inventory")?;
    inventory["expressions"] = normalized_expressions(db, &schema).await?;
    // pg_get_* emits schema qualifiers when required by the active search path.
    // The two inventories use different schema names but identical definitions.
    serde_json::from_str(&inventory.to_string().replace(&format!("{schema}."), ""))
        .map_err(|error| DbErr::Custom(format!("invalid schema inventory: {error}")))
}

/// PostgreSQL can rewrite equivalent casts and nested ANDs when a dumped CHECK
/// is parsed again. Parse both inventories once through temporary views, using
/// this server's parser, instead of weakening comparisons with text rewrites.
async fn normalized_expressions<C: ConnectionTrait>(db: &C, schema: &str) -> Result<Value, DbErr> {
    let rows = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"
        SELECT r.relname, c.conname AS name, 'constraint' AS kind,
               pg_get_expr(c.conbin, c.conrelid) AS expression
        FROM pg_constraint c JOIN pg_class r ON r.oid = c.conrelid
        JOIN pg_namespace n ON n.oid = r.relnamespace
        WHERE n.nspname = current_schema() AND r.relname <> 'seaql_migrations'
          AND c.contype = 'c'
        UNION ALL
        SELECT r.relname, idx.relname, 'index', pg_get_expr(i.indpred, i.indrelid)
        FROM pg_index i JOIN pg_class r ON r.oid = i.indrelid
        JOIN pg_class idx ON idx.oid = i.indexrelid
        JOIN pg_namespace n ON n.oid = r.relnamespace
        WHERE n.nspname = current_schema() AND r.relname <> 'seaql_migrations'
          AND i.indpred IS NOT NULL
        ORDER BY relname, kind, name
    "#,
        ))
        .await?;
    let mut tables = std::collections::BTreeMap::<String, Vec<(String, String, String)>>::new();
    for row in rows {
        tables
            .entry(row.try_get("", "relname")?)
            .or_default()
            .push((
                row.try_get("", "kind")?,
                row.try_get("", "name")?,
                row.try_get("", "expression")?,
            ));
    }
    let mut normalized = serde_json::Map::new();
    for (table, expressions) in tables {
        let columns = expressions
            .iter()
            .enumerate()
            .map(|(index, (_, _, expression))| format!("({expression}) AS expression_{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        db.execute_unprepared(&format!(
            "CREATE TEMP VIEW scope_baseline_expression_inventory AS SELECT {columns} FROM {}.{} AS source",
            quote_identifier(schema), quote_identifier(&table),
        )).await?;
        let definition = db.query_one(Statement::from_string(DatabaseBackend::Postgres,
            "SELECT pg_get_viewdef('pg_temp.scope_baseline_expression_inventory'::regclass, false) AS definition",
        )).await?.ok_or_else(|| DbErr::Custom("PostgreSQL did not normalize schema expressions".into()))?
            .try_get::<String>("", "definition")?;
        db.execute_unprepared("DROP VIEW pg_temp.scope_baseline_expression_inventory")
            .await?;
        let names = expressions
            .into_iter()
            .map(|(kind, name, _)| (kind, name))
            .collect::<Vec<_>>();
        normalized.insert(
            table,
            serde_json::json!({"names": names, "definition": definition}),
        );
    }
    Ok(Value::Object(normalized))
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
