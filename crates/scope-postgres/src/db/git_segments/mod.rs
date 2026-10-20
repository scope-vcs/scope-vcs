mod ledger;
mod pool;
mod references;
mod retirement;
mod spans;

pub use pool::GitPoolPacks;
pub(super) use pool::{register_git_pool_pack, retire_request_pool_packs};
pub(super) use references::{insert_git_segment_references, release_git_segment_references};
pub(super) use retirement::retire_git_segment;
pub(super) use spans::{load_git_pack_spans, publish_git_segment};

use super::{RepositoryStore, content_fences};
use crate::error::PostgresError;
use sqlx::{PgPool, Postgres, pool::PoolConnection, postgres::PgPoolOptions};
use std::time::Duration;

const WRITE_LEASE_SESSION_IDLE: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(in crate::db) struct GitWriteLeaseSessions {
    pool: PgPool,
}

impl GitWriteLeaseSessions {
    pub(in crate::db) fn lazy(
        database_url: &str,
        max_sessions: u32,
    ) -> Result<Self, PostgresError> {
        PgPoolOptions::new()
            .max_connections(max_sessions)
            .min_connections(0)
            .idle_timeout(WRITE_LEASE_SESSION_IDLE)
            .connect_lazy(database_url)
            .map(|pool| Self { pool })
            .map_err(PostgresError::internal)
    }
}

pub struct RepositoryGitWriteLease {
    session: Option<PoolConnection<Postgres>>,
    key: i64,
}

impl RepositoryGitWriteLease {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "release"))]
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
                "repository Git write lease was not held at release; closing its Postgres session"
            ),
            Err(error) => tracing::warn!(
                error = %error,
                "failed to release repository Git write lease; closing its Postgres session"
            ),
        }
    }
}

impl Drop for RepositoryGitWriteLease {
    fn drop(&mut self) {
        if let Some(session) = self.session.as_mut() {
            session.close_on_drop();
        }
    }
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "acquire_git_write_lease"))]
    pub async fn acquire_git_write_lease(
        &self,
        repo_id: &str,
    ) -> Result<RepositoryGitWriteLease, PostgresError> {
        if repo_id.trim().is_empty() {
            return Err(PostgresError::internal_message(
                "repository Git write lease requires a repository id",
            ));
        }
        let schema = content_fences::current_schema(self.db.as_ref()).await?;
        let key = sea_orm::sqlx::postgres::PgAdvisoryLock::new(format!(
            "scope:git-write:{schema}:{repo_id}"
        ))
        .key()
        .as_bigint()
        .expect("string-derived advisory locks use one bigint key");
        let session = self
            .git_write_lease_sessions
            .pool
            .acquire()
            .await
            .map_err(PostgresError::internal)?;
        let mut lease = RepositoryGitWriteLease {
            session: Some(session),
            key,
        };
        let session = lease
            .session
            .as_mut()
            .expect("a new repository Git write lease holds its session");
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(key)
            .execute(&mut **session)
            .await
            .map_err(PostgresError::internal)?;
        Ok(lease)
    }
}

#[cfg(test)]
mod tests;
