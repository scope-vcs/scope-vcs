use super::MetadataStore;
use crate::error::PostgresError;
use sea_orm::{
    AccessMode, ConnectOptions, DatabaseConnection, DatabaseTransaction, IsolationLevel,
    SqlxPostgresConnector, TransactionTrait,
};
use sqlx::{AssertSqlSafe, Connection as _, PgConnection};
use std::sync::Arc;

const WRITER_FENCE_KEY: &str = "scope:metadata-writers";
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

pub async fn verify_writer_fence_available(database_url: String) -> anyhow::Result<()> {
    ExclusiveWriterFence::acquire(&database_url)
        .await?
        .release()
        .await
}

pub async fn terminate_metadata_writer_sessions(database_url: String) -> anyhow::Result<u64> {
    let mut connection = PgConnection::connect(&database_url).await?;
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
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "acquire"))]
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

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "release"))]
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
    max_connections: u32,
) -> anyhow::Result<DatabaseConnection> {
    let mut options = ConnectOptions::new(database_url.to_string());
    options.min_connections(1);
    options.max_connections(max_connections);
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
