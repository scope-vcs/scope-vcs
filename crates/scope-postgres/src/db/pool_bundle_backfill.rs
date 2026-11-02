use super::{
    RepositoryStore, content_fences,
    git_segments::register_git_pool_pack,
    integer_columns::u64_to_i64,
    locks::{acquire_aggregate_lock, acquire_shared_repository_lock},
};
use crate::error::PostgresError;
use scope_domain::{
    content::SourceBlob,
    repository::{
        RepositoryIncarnation,
        bundle_backfill::{
            BACKFILL_CADENCE, BACKFILL_FAILURE_RETRY, BACKFILL_IDLE_CADENCE, BACKFILL_LEASE,
            VerifiedPoolRestore, backfill_failure_reason,
        },
        git::GitPoolPack,
    },
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult, Statement, TransactionTrait,
};
use serde::Serialize;

const SOURCES: &str = "WITH sources AS (
    SELECT 'revision:' || revision.id AS source_id, request.repo_id, request.id AS request_id,
        revision.new_head_oid AS head_oid, revision.old_head_oid, revision.git_snapshot, revision.position, request.audience, request.stack_depth
    FROM scope_request_revisions revision JOIN scope_requests request ON request.id = revision.request_id
    UNION ALL
    SELECT 'request:' || request.id || ':' || request.head_oid || ':' || md5(request.git_snapshot::text),
        request.repo_id, request.id, request.head_oid, request.review_base_oid, request.git_snapshot, 9223372036854775807, request.audience, request.stack_depth
    FROM scope_requests request WHERE request.git_snapshot IS NOT NULL
        AND NOT EXISTS (SELECT 1 FROM scope_request_revisions revision
            WHERE revision.request_id = request.id AND revision.new_head_oid = request.head_oid)
), coverage AS (
    SELECT sources.*, EXISTS (SELECT 1 FROM scope_pool_bundle_backfill_verifications verified
        JOIN scope_repositories repo ON repo.id = sources.repo_id
        WHERE verified.source_id = sources.source_id AND verified.head_oid = sources.head_oid
            AND verified.old_head_oid = sources.old_head_oid AND verified.audience = sources.audience
            AND verified.incarnation_id = repo.incarnation_id AND verified.repo_config = repo.repo_config
            AND NOT EXISTS (SELECT 1 FROM unnest(verified.pack_ids) AS required(id)
                WHERE NOT EXISTS (SELECT 1 FROM scope_git_pool_packs pack WHERE pack.id = required.id))
            AND NOT EXISTS (SELECT 1 FROM jsonb_each_text(verified.reader_request_capacities) capacity
                WHERE (SELECT count(*) FROM scope_git_pool_packs pack
                    WHERE pack.request_id = capacity.key
                        AND pack.view_id IS NOT DISTINCT FROM NULLIF(sources.audience, 'private'))
                    > capacity.value::bigint)) AS done
    FROM sources
)";

#[derive(Clone, Debug, FromQueryResult)]
pub struct BundleBackfillSource {
    pub source_id: String,
    pub repo_id: String,
    pub request_id: String,
    pub head_oid: String,
    pub old_head_oid: String,
    pub audience: String,
    pub git_snapshot: sea_orm::JsonValue,
}

impl BundleBackfillSource {
    pub fn snapshot(&self) -> Result<SourceBlob, PostgresError> {
        serde_json::from_value(self.git_snapshot.clone()).map_err(PostgresError::internal)
    }
}

pub struct BundleBackfillVerification {
    pub restored: VerifiedPoolRestore,
    pub pack_ids: Vec<String>,
    pub request_pack_ids: Vec<String>,
    pub reader_request_capacities: std::collections::BTreeMap<String, usize>,
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
    pub coverage: &'static str,
    pub excluded_objects: [&'static str; 2],
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
        let available = self.db.query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT singleton FROM scope_pool_bundle_backfill_lease WHERE singleton AND available_at_unix <= $1
                AND (needs_scan OR EXISTS (SELECT 1 FROM scope_pool_bundle_backfill_failures WHERE retry_at_unix <= $1))",
            [u64_to_i64(now_unix, "backfill claim time")?.into()],
        )).await?.is_some();
        if !available {
            return Ok(None);
        }
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
                    u64_to_i64(
                        now_unix.saturating_add(BACKFILL_LEASE.as_secs()),
                        "backfill lease expiry",
                    )?
                    .into(),
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
                ORDER BY coverage.repo_id, coverage.stack_depth, coverage.request_id, coverage.position, coverage.source_id LIMIT 1"),
            [u64_to_i64(now_unix, "backfill claim time")?.into(), super::entities::encode_enum(scope_domain::repository::RepoLifecycleState::Ready)?.into()],
        )).one(&tx).await?;
        if source.is_none() {
            tx.execute_unprepared(
                "UPDATE scope_pool_bundle_backfill_lease SET needs_scan = false WHERE singleton",
            )
            .await?;
        }
        tx.commit().await?;
        match source {
            Some(source) => Ok(Some(BundleBackfillClaim {
                source,
                token: token.to_string(),
                _session: session,
            })),
            None => {
                self.release_bundle_backfill_lease(
                    token,
                    now_unix,
                    BACKFILL_IDLE_CADENCE.as_secs(),
                )
                .await?;
                Ok(None)
            }
        }
    }

    pub async fn complete_pool_bundle_backfill(
        &self,
        claim: &BundleBackfillClaim,
        incarnation: &RepositoryIncarnation,
        generation: &str,
        pack: Option<&GitPoolPack>,
        verification: &BundleBackfillVerification,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await?;
        acquire_shared_repository_lock(&tx, incarnation.repository_id()).await?;
        acquire_aggregate_lock(&tx, "request", &claim.source.request_id).await?;
        let owned = tx.query_one_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "SELECT token FROM scope_pool_bundle_backfill_lease WHERE singleton AND token = $1 FOR UPDATE",
            [claim.token.clone().into()],
        )).await?.is_some();
        if !owned {
            return Ok(false);
        }
        let Some(repo) =
            super::entities::repository::Entity::find_by_id(incarnation.repository_id())
                .one(&tx)
                .await?
        else {
            return Ok(false);
        };
        let config: scope_domain::repo_config::RepoConfig =
            serde_json::from_value(repo.repo_config.clone()).map_err(PostgresError::internal)?;
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
        if request.view.as_str() != claim.source.audience {
            return Ok(false);
        }
        let mut revision_request = request;
        revision_request.head_oid = claim.source.head_oid.clone();
        let scope = scope_domain::repository::git::RequestPackScope::for_view(
            &revision_request.view,
            config.views.full(),
        );
        if pack.is_some_and(|pack| {
            !pack.belongs_to_request(&revision_request)
                || !matches!(&pack.owner, scope_domain::repository::git::GitPoolPackOwner::Request { scope: pack_scope, .. } if pack_scope == &scope)
        }) {
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
        if !missing {
            return Ok(false);
        }
        if let Some(pack) = pack {
            register_git_pool_pack(&tx, &claim.source.repo_id, pack, now_unix).await?;
        }
        let current_packs = super::git_segments::pool::request_pack_ids(
            &tx,
            &claim.source.request_id,
            &revision_request.view,
        )
        .await?;
        if current_packs != verification.request_pack_ids {
            return Ok(false);
        }
        let present = tx.query_one_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_git_pool_packs WHERE repo_id = $1 AND id = ANY($2)",
            [claim.source.repo_id.clone().into(), verification.pack_ids.clone().into()],
        )).await?.unwrap().try_get::<i64>("", "count")?;
        if present as usize != verification.pack_ids.len() {
            return Ok(false);
        }
        tx.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "INSERT INTO scope_pool_bundle_backfill_verifications
                (source_id, request_id, head_oid, old_head_oid, audience, incarnation_id, repo_config,
                    pack_ids, request_pack_ids, reader_request_capacities, verified_at_unix)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (source_id) DO UPDATE SET head_oid = EXCLUDED.head_oid,
                old_head_oid = EXCLUDED.old_head_oid, audience = EXCLUDED.audience,
                incarnation_id = EXCLUDED.incarnation_id, repo_config = EXCLUDED.repo_config,
                pack_ids = EXCLUDED.pack_ids, request_pack_ids = EXCLUDED.request_pack_ids,
                reader_request_capacities = EXCLUDED.reader_request_capacities, verified_at_unix = EXCLUDED.verified_at_unix",
            [claim.source.source_id.clone().into(), claim.source.request_id.clone().into(),
                claim.source.head_oid.clone().into(), claim.source.old_head_oid.clone().into(),
                revision_request.view.as_str().into(), incarnation.incarnation_id().into(), repo.repo_config.into(),
                verification.pack_ids.clone().into(), verification.request_pack_ids.clone().into(),
                serde_json::to_value(&verification.reader_request_capacities).map_err(PostgresError::internal)?.into(),
                u64_to_i64(now_unix, "backfill verification time")?.into()],
        )).await?;
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_pool_bundle_backfill_failures WHERE source_id = $1",
            [claim.source.source_id.clone().into()],
        ))
        .await?;
        tx.commit().await?;
        Ok(true)
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
                backfill_failure_reason(reason).into(),
                u64_to_i64(now_unix, "backfill failure time")?.into(),
                u64_to_i64(now_unix.saturating_add(BACKFILL_FAILURE_RETRY.as_secs()), "backfill retry time")?.into()],
        )).await?;
        Ok(())
    }

    pub async fn release_pool_bundle_backfill(
        &self,
        claim: BundleBackfillClaim,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        self.release_bundle_backfill_lease(&claim.token, now_unix, BACKFILL_CADENCE.as_secs())
            .await
    }

    async fn release_bundle_backfill_lease(
        &self,
        token: &str,
        now_unix: u64,
        delay: u64,
    ) -> Result<(), PostgresError> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_pool_bundle_backfill_lease SET token = NULL, available_at_unix = $2
             WHERE singleton AND token = $1",
                [
                    token.into(),
                    u64_to_i64(now_unix.saturating_add(delay), "backfill next pass")?.into(),
                ],
            ))
            .await?;
        Ok(())
    }
}

pub async fn pool_bundle_backfill_report_for_maintenance(
    database_url: String,
) -> anyhow::Result<PoolBundleBackfillReport> {
    let db = sea_orm::Database::connect(database_url).await?;
    Ok(report(&db).await?)
}

pub(super) async fn report(
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
        coverage: "Stored request revisions and request snapshots without a revision row; done requires a durable pool-only restore verification of head, old head and connected history within reader limits",
        excluded_objects: [
            "RunSource::EphemeralGitBundle objects retained by run_source references",
            "RunSource::RequestGitSnapshot objects retained by run_source references, even when shared with a request revision",
        ],
        total,
        failures,
        repositories,
    })
}

#[cfg(test)]
mod tests;
