//! Metadata connection setup and the advisory writer fence that guards it.

use super::MetadataStore;
use crate::error::PostgresError;
use sea_orm::{
    AccessMode, ConnectOptions, DatabaseConnection, DatabaseTransaction, IsolationLevel,
    SqlxPostgresConnector, TransactionTrait,
};
use sqlx::{AssertSqlSafe, Connection as _, PgConnection};
use std::sync::Arc;

const WRITER_FENCE_KEY: &str = "scope:metadata-writers";

pub(super) async fn connect_postgres_store(database_url: String) -> anyhow::Result<MetadataStore> {
    let options = database_url.parse()?;
    connect_postgres_store_with_options(database_url, options).await
}

pub(super) async fn connect_postgres_store_with_options(
    database_url: String,
    connection_options: sqlx::postgres::PgConnectOptions,
) -> anyhow::Result<MetadataStore> {
    let database_url = Arc::<str>::from(database_url);
    let db = connect_writer_database(&database_url, connection_options).await?;
    if let Err(error) = crate::migrations::assert_exact_state(&db).await {
        // A rejected startup must release its writer fence before maintenance retries.
        db.close().await?;
        return Err(error.into());
    }
    Ok(MetadataStore {
        db: Arc::new(db),
        postgres_database_url: Some(database_url),
        #[cfg(any(test, feature = "test-support"))]
        _test_schema: None,
    })
}

pub async fn verify_writer_fence_available(database_url: String) -> anyhow::Result<()> {
    ExclusiveWriterFence::acquire(&database_url)
        .await?
        .release()
        .await
}

pub async fn terminate_metadata_writer_sessions(database_url: String) -> anyhow::Result<u64> {
    let mut connection = PgConnection::connect(&database_url).await?;
    // Only the fixed fence key is interpolated; database names remain SQL values.
    let terminated: Vec<bool> = sqlx::query_scalar(AssertSqlSafe(format!(
        "WITH fence AS (
            SELECT hashtextextended(
                '{WRITER_FENCE_KEY}:' || current_database() || ':' || current_schema(),
                0
            ) AS key
        )
        SELECT pg_terminate_backend(locks.pid)
        FROM pg_locks locks
        CROSS JOIN fence
        WHERE locks.locktype = 'advisory'
            AND locks.mode = 'ShareLock'
            AND locks.granted
            AND locks.objsubid = 1
            AND locks.classid::bigint = ((fence.key >> 32) & 4294967295)
            AND locks.objid::bigint = (fence.key & 4294967295)
            AND locks.pid <> pg_backend_pid()"
    )))
    .fetch_all(&mut connection)
    .await?;
    connection.close().await?;
    Ok(terminated.into_iter().filter(|value| *value).count() as u64)
}

pub struct ExclusiveWriterFence {
    connection: PgConnection,
}

impl ExclusiveWriterFence {
    pub async fn acquire(database_url: &str) -> anyhow::Result<Self> {
        let mut connection = PgConnection::connect(database_url).await?;
        let acquired: bool = sqlx::query_scalar(AssertSqlSafe(writer_fence_statement(
            "pg_try_advisory_lock",
        )))
        .fetch_one(&mut connection)
        .await?;
        if !acquired {
            anyhow::bail!(
                "maintenance migration refused: a metadata writer still holds the database fence"
            );
        }
        Ok(Self { connection })
    }

    pub async fn release(mut self) -> anyhow::Result<()> {
        sqlx::query(AssertSqlSafe(writer_fence_statement("pg_advisory_unlock")))
            .execute(&mut self.connection)
            .await?;
        self.connection.close().await?;
        Ok(())
    }
}

pub(super) async fn connect_writer_database(
    database_url: &str,
    connection_options: sqlx::postgres::PgConnectOptions,
) -> anyhow::Result<DatabaseConnection> {
    let mut options = ConnectOptions::new(database_url.to_string());
    options.min_connections(1);
    let fence_statement = writer_fence_statement("pg_advisory_lock_shared");
    let pool = options
        .sqlx_pool_options()
        .after_connect(move |connection, _| {
            let fence_statement = fence_statement.clone();
            Box::pin(async move {
                sqlx::query(AssertSqlSafe(fence_statement))
                    .execute(connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect_with(connection_options)
        .await?;
    Ok(SqlxPostgresConnector::from_sqlx_postgres_pool(pool))
}

fn writer_fence_statement(function: &str) -> String {
    format!(
        "SELECT {function}(
            hashtextextended(
                '{WRITER_FENCE_KEY}:' || current_database() || ':' || current_schema(),
                0
            )
        ) AS acquired"
    )
}

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
