use super::MetadataStore;
use crate::error::PostgresError;
use sea_orm::{
    AccessMode, ConnectOptions, DatabaseConnection, DatabaseTransaction, IsolationLevel,
    SqlxPostgresConnector, TransactionTrait,
};
use std::sync::Arc;

pub const DEFAULT_METADATA_POOL_MAX_CONNECTIONS: u32 = 10;

pub(super) async fn connect_postgres_store(
    database_url: String,
    max_connections: u32,
) -> anyhow::Result<MetadataStore> {
    let options = database_url.parse()?;
    connect_postgres_store_with_options(database_url, options, max_connections).await
}

pub(super) async fn connect_postgres_store_with_options(
    database_url: String,
    connection_options: sqlx::postgres::PgConnectOptions,
    max_connections: u32,
) -> anyhow::Result<MetadataStore> {
    let database_url = Arc::<str>::from(database_url);
    let db = connect_writer_database(&database_url, connection_options, max_connections).await?;
    if let Err(error) = crate::migrations::assert_exact_state(&db).await {
        db.close().await?;
        return Err(error.into());
    }
    Ok(MetadataStore {
        db: Arc::new(db),
        merge_turn_sessions: super::merge_turns::MergeTurnSessions::new(Arc::clone(&database_url)),
        postgres_database_url: Some(database_url),
        repo_change_notifier: Default::default(),
        #[cfg(any(test, feature = "test-support"))]
        _test_schema: None,
    })
}

pub(super) async fn connect_writer_database(
    database_url: &str,
    connection_options: sqlx::postgres::PgConnectOptions,
    max_connections: u32,
) -> anyhow::Result<DatabaseConnection> {
    let mut options = ConnectOptions::new(database_url.to_string());
    options.min_connections(1);
    options.max_connections(max_connections);
    let pool = options
        .sqlx_pool_options()
        .connect_with(connection_options.options([("jit", "off")]))
        .await?;
    Ok(SqlxPostgresConnector::from_sqlx_postgres_pool(pool))
}

#[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "begin_metadata_read_snapshot"))]
pub(super) async fn begin_metadata_read_snapshot(
    db: &DatabaseConnection,
) -> Result<DatabaseTransaction, PostgresError> {
    db.begin_with_config(
        Some(IsolationLevel::RepeatableRead),
        Some(AccessMode::ReadOnly),
    )
    .await
    .map_err(PostgresError::internal)
}
