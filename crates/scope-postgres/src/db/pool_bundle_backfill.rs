use super::{
    RepositoryStore, content_fences, git_segments::register_git_pool_pack,
    integer_columns::u64_to_i64, locks::acquire_shared_repository_lock,
};
use crate::error::PostgresError;
use scope_domain::{
    content::SourceBlob,
    repository::{RepositoryIncarnation, git::GitPoolPack},
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult, Statement, TransactionTrait,
};
use serde::Serialize;

const SOURCES: &str = "WITH sources AS (
    SELECT 'revision:' || revision.id AS source_id, request.repo_id, request.id AS request_id,
        revision.new_head_oid AS head_oid, revision.git_snapshot, revision.position, request.audience
    FROM scope_request_revisions revision JOIN scope_requests request ON request.id = revision.request_id
    UNION ALL
    SELECT 'request:' || request.id || ':' || request.head_oid || ':' || md5(request.git_snapshot::text),
        request.repo_id, request.id, request.head_oid, request.git_snapshot, 9223372036854775807, request.audience
    FROM scope_requests request WHERE request.git_snapshot IS NOT NULL
        AND NOT EXISTS (SELECT 1 FROM scope_request_revisions revision
            WHERE revision.request_id = request.id AND revision.new_head_oid = request.head_oid)
), coverage AS (
    SELECT sources.*, EXISTS (SELECT 1 FROM scope_git_pool_packs pack
        WHERE pack.request_id = sources.request_id AND pack.repo_id = sources.repo_id
            AND pack.head_oid = sources.head_oid
            AND pack.view_id IS NOT DISTINCT FROM NULLIF(sources.audience, 'private')) AS done
    FROM sources
)";

#[derive(Clone, Debug, FromQueryResult)]
pub struct BundleBackfillSource {
    pub source_id: String,
    pub repo_id: String,
    pub request_id: String,
    pub head_oid: String,
    pub git_snapshot: sea_orm::JsonValue,
}

impl BundleBackfillSource {
    pub fn snapshot(&self) -> Result<SourceBlob, PostgresError> {
        serde_json::from_value(self.git_snapshot.clone()).map_err(PostgresError::internal)
    }
}

pub struct BundleBackfillClaim {
    pub source: BundleBackfillSource,
    token: String,
    _session: sqlx::PgConnection,
}

#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct BundleBackfillCounts {
    pub remaining: u64,
    pub needing_backfill: u64,
    pub done: u64,
    pub failed: u64,
}

#[derive(Debug, Serialize)]
pub struct RepositoryBundleBackfillReport {
    pub repo_id: String,
    #[serde(flatten)]
    pub counts: BundleBackfillCounts,
}

#[derive(Debug, Serialize)]
pub struct BundleBackfillFailure {
    pub repo_id: String,
    pub source_id: String,
    pub head_oid: String,
    pub reason: String,
    pub attempted_at_unix: i64,
}

#[derive(Debug, Serialize)]
pub struct PoolBundleBackfillReport {
    pub repositories: Vec<RepositoryBundleBackfillReport>,
    pub total: BundleBackfillCounts,
    pub failures: Vec<BundleBackfillFailure>,
}

impl RepositoryStore {
    pub async fn claim_pool_bundle_backfill(
        &self,
        token: &str,
        now_unix: u64,
    ) -> Result<Option<BundleBackfillClaim>, PostgresError> {
        let schema = content_fences::current_schema(self.db.as_ref()).await?;
        let mut session = content_fences::dedicated_fence_connection(
            self.postgres_database_url.as_deref(),
            &schema,
        )
        .await?;
        let key = sea_orm::sqlx::postgres::PgAdvisoryLock::new(format!(
            "scope:pool-bundle-backfill:{schema}"
        ))
        .key()
        .as_bigint()
        .expect("string-derived advisory lock uses bigint");
        let held: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(key)
            .fetch_one(&mut session)
            .await
            .map_err(PostgresError::internal)?;
        if !held {
            return Ok(None);
        }
        let tx = self.db.begin().await?;
        let claimed = tx
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_pool_bundle_backfill_lease SET token = $1, available_at_unix = $2
             WHERE singleton AND available_at_unix <= $3",
                [
                    token.into(),
                    u64_to_i64(now_unix.saturating_add(600), "backfill lease expiry")?.into(),
                    u64_to_i64(now_unix, "backfill claim time")?.into(),
                ],
            ))
            .await?
            .rows_affected()
            == 1;
        if !claimed {
            return Ok(None);
        }
        let source = BundleBackfillSource::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!("{SOURCES} SELECT coverage.* FROM coverage
                JOIN scope_repositories repo ON repo.id = coverage.repo_id
                LEFT JOIN scope_pool_bundle_backfill_failures failure USING (source_id)
                WHERE NOT done AND repo.publication_state = $2
                    AND (failure.retry_at_unix IS NULL OR failure.retry_at_unix <= $1)
                ORDER BY coverage.repo_id, coverage.request_id, coverage.position, coverage.source_id LIMIT 1"),
            [u64_to_i64(now_unix, "backfill claim time")?.into(), super::entities::encode_enum(scope_domain::repository::RepoLifecycleState::Ready)?.into()],
        )).one(&tx).await?;
        tx.commit().await?;
        match source {
            Some(source) => Ok(Some(BundleBackfillClaim {
                source,
                token: token.to_string(),
                _session: session,
            })),
            None => {
                self.release_bundle_backfill_lease(token, now_unix).await?;
                Ok(None)
            }
        }
    }

    pub async fn complete_pool_bundle_backfill(
        &self,
        claim: &BundleBackfillClaim,
        incarnation: &RepositoryIncarnation,
        generation: &str,
        pack: &GitPoolPack,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await?;
        let owned = tx.query_one_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "SELECT token FROM scope_pool_bundle_backfill_lease WHERE singleton AND token = $1 FOR UPDATE",
            [claim.token.clone().into()],
        )).await?.is_some();
        if !owned {
            return Ok(false);
        }
        acquire_shared_repository_lock(&tx, incarnation.repository_id()).await?;
        let Some(repo) =
            super::entities::repository::Entity::find_by_id(incarnation.repository_id())
                .one(&tx)
                .await?
        else {
            return Ok(false);
        };
        let config: scope_domain::repo_config::RepoConfig =
            serde_json::from_value(repo.repo_config).map_err(PostgresError::internal)?;
        if repo.incarnation_id != incarnation.incarnation_id()
            || config.object_generation() != generation
        {
            return Ok(false);
        }
        let Some(request) =
            super::request_rows::request_by_id(&tx, &claim.source.request_id).await?
        else {
            return Ok(false);
        };
        let mut revision_request = request;
        revision_request.head_oid = claim.source.head_oid.clone();
        if !pack.belongs_to_request(&revision_request) {
            return Err(PostgresError::internal_message(
                "backfill pack does not belong to revision",
            ));
        }
        let missing = tx
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                format!(
                    "{SOURCES} SELECT source_id FROM coverage WHERE source_id = $1 AND NOT done"
                ),
                [claim.source.source_id.clone().into()],
            ))
            .await?
            .is_some();
        if missing {
            register_git_pool_pack(&tx, &claim.source.repo_id, pack, now_unix).await?;
        }
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_pool_bundle_backfill_failures WHERE source_id = $1",
            [claim.source.source_id.clone().into()],
        ))
        .await?;
        tx.commit().await?;
        Ok(missing)
    }

    pub async fn fail_pool_bundle_backfill(
        &self,
        claim: &BundleBackfillClaim,
        reason: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        self.db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "INSERT INTO scope_pool_bundle_backfill_failures
                (source_id, request_id, reason, attempted_at_unix, retry_at_unix)
             SELECT $1, id, $3, $4, $5 FROM scope_requests WHERE id = $2
             ON CONFLICT (source_id) DO UPDATE SET reason = EXCLUDED.reason,
                attempted_at_unix = EXCLUDED.attempted_at_unix, retry_at_unix = EXCLUDED.retry_at_unix",
            [claim.source.source_id.clone().into(), claim.source.request_id.clone().into(),
                reason.chars().take(2000).collect::<String>().into(),
                u64_to_i64(now_unix, "backfill failure time")?.into(),
                u64_to_i64(now_unix.saturating_add(86400), "backfill retry time")?.into()],
        )).await?;
        Ok(())
    }

    pub async fn release_pool_bundle_backfill(
        &self,
        claim: BundleBackfillClaim,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        self.release_bundle_backfill_lease(&claim.token, now_unix)
            .await
    }

    async fn release_bundle_backfill_lease(
        &self,
        token: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_pool_bundle_backfill_lease SET token = NULL, available_at_unix = $2
             WHERE singleton AND token = $1",
                [
                    token.into(),
                    u64_to_i64(now_unix.saturating_add(30), "backfill next pass")?.into(),
                ],
            ))
            .await?;
        Ok(())
    }

    pub async fn pool_bundle_backfill_report(
        &self,
    ) -> Result<PoolBundleBackfillReport, PostgresError> {
        report(self.db.as_ref()).await
    }
}

pub async fn pool_bundle_backfill_report_for_maintenance(
    database_url: String,
) -> anyhow::Result<PoolBundleBackfillReport> {
    let db = sea_orm::Database::connect(database_url).await?;
    Ok(report(&db).await?)
}

async fn report(
    db: &sea_orm::DatabaseConnection,
) -> Result<PoolBundleBackfillReport, PostgresError> {
    let tx = db
        .begin_with_config(
            Some(sea_orm::IsolationLevel::RepeatableRead),
            Some(sea_orm::AccessMode::ReadOnly),
        )
        .await?;
    let rows = tx
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!(
                "{SOURCES} SELECT repo_id,
            count(*) FILTER (WHERE NOT done) AS remaining,
            count(*) FILTER (WHERE NOT done AND reason IS NULL) AS needing_backfill,
            count(*) FILTER (WHERE done) AS done,
            count(*) FILTER (WHERE NOT done AND reason IS NOT NULL) AS failed
            FROM coverage LEFT JOIN scope_pool_bundle_backfill_failures USING (source_id)
            GROUP BY repo_id ORDER BY repo_id"
            ),
        ))
        .await?;
    let mut repositories = Vec::new();
    let mut total = BundleBackfillCounts::default();
    for row in rows {
        let counts = BundleBackfillCounts {
            remaining: super::integer_columns::i64_to_u64(
                row.try_get("", "remaining")?,
                "backfill remaining count",
            )?,
            needing_backfill: super::integer_columns::i64_to_u64(
                row.try_get("", "needing_backfill")?,
                "backfill pending count",
            )?,
            done: super::integer_columns::i64_to_u64(
                row.try_get("", "done")?,
                "backfill done count",
            )?,
            failed: super::integer_columns::i64_to_u64(
                row.try_get("", "failed")?,
                "backfill failed count",
            )?,
        };
        total.remaining += counts.remaining;
        total.needing_backfill += counts.needing_backfill;
        total.done += counts.done;
        total.failed += counts.failed;
        repositories.push(RepositoryBundleBackfillReport {
            repo_id: row.try_get("", "repo_id")?,
            counts,
        });
    }
    let failures = tx
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!(
                "{SOURCES} SELECT repo_id, source_id, head_oid, reason, attempted_at_unix
            FROM coverage JOIN scope_pool_bundle_backfill_failures USING (source_id)
            WHERE NOT done ORDER BY repo_id, source_id"
            ),
        ))
        .await?
        .into_iter()
        .map(|row| {
            Ok(BundleBackfillFailure {
                repo_id: row.try_get("", "repo_id")?,
                source_id: row.try_get("", "source_id")?,
                head_oid: row.try_get("", "head_oid")?,
                reason: row.try_get("", "reason")?,
                attempted_at_unix: row.try_get("", "attempted_at_unix")?,
            })
        })
        .collect::<Result<Vec<_>, PostgresError>>()?;
    tx.commit().await?;
    Ok(PoolBundleBackfillReport {
        total,
        failures,
        repositories,
    })
}

#[cfg(test)]
mod tests;
