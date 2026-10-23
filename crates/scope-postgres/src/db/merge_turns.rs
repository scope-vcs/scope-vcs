use super::{RepositoryStore, content_fences};
use crate::error::PostgresError;
use sqlx::{PgPool, Postgres, pool::PoolConnection, postgres::PgPoolOptions};
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

const MERGE_TURN_SESSIONS: u32 = 16;
const MERGE_TURN_SESSION_IDLE: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(in crate::db) struct MergeTurnSessions {
    database_url: Arc<str>,
    pool: Arc<OnceLock<PgPool>>,
}

impl MergeTurnSessions {
    pub(in crate::db) fn new(database_url: Arc<str>) -> Self {
        Self {
            database_url,
            pool: Arc::default(),
        }
    }

    fn pool(&self) -> Result<&PgPool, PostgresError> {
        if let Some(pool) = self.pool.get() {
            return Ok(pool);
        }
        let pool = PgPoolOptions::new()
            .max_connections(MERGE_TURN_SESSIONS)
            .min_connections(0)
            .idle_timeout(MERGE_TURN_SESSION_IDLE)
            .connect_lazy(&self.database_url)
            .map_err(PostgresError::internal)?;
        Ok(self.pool.get_or_init(|| pool))
    }
}

pub struct RepositoryMergeTurn {
    session: Option<PoolConnection<Postgres>>,
    key: i64,
}

impl RepositoryMergeTurn {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "release_merge_turn"))]
    pub async fn release(mut self) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        match sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock($1)")
            .bind(self.key)
            .fetch_one(&mut **session)
            .await
        {
            Ok(true) => {
                self.session.take();
            }
            Ok(false) => tracing::warn!(
                "repository merge turn was not held at release; closing its Postgres session"
            ),
            Err(error) => tracing::warn!(
                error = %error,
                "failed to release repository merge turn; closing its Postgres session"
            ),
        }
    }
}

impl Drop for RepositoryMergeTurn {
    fn drop(&mut self) {
        if let Some(session) = self.session.as_mut() {
            session.close_on_drop();
        }
    }
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "acquire_merge_turn"))]
    pub async fn acquire_merge_turn(
        &self,
        repo_id: &str,
    ) -> Result<RepositoryMergeTurn, PostgresError> {
        if repo_id.trim().is_empty() {
            return Err(PostgresError::internal_message(
                "repository merge turn requires a repository id",
            ));
        }
        let schema = content_fences::current_schema(self.db.as_ref()).await?;
        let key = sea_orm::sqlx::postgres::PgAdvisoryLock::new(format!(
            "scope:merge-turn:{schema}:{repo_id}"
        ))
        .key()
        .as_bigint()
        .expect("string-derived advisory locks use one bigint key");
        let session = self
            .merge_turn_sessions
            .pool()?
            .acquire()
            .await
            .map_err(PostgresError::internal)?;
        let mut turn = RepositoryMergeTurn {
            session: Some(session),
            key,
        };
        let session = turn
            .session
            .as_mut()
            .expect("a new repository merge turn holds its session");
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(key)
            .execute(&mut **session)
            .await
            .map_err(PostgresError::internal)?;
        Ok(turn)
    }
}

#[cfg(test)]
mod tests;
