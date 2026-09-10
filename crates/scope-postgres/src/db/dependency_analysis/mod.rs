use super::{
    GeneratedIdKind, GeneratedIdSource, JobStore, RepositoryStore, acquire_aggregate_lock,
    decode_json, generated_ids::generate_id, git_segments::load_git_pack_spans,
};
use crate::error::PostgresError;
use scope_domain::{
    content::SourceBlob,
    dependency_analysis::{
        AnalyzerOutput, DEPENDENCY_ANALYZER_VERSION, StoredDependencyAnalysis,
        evaluate_dependency_analysis,
    },
    policy::ScopePath,
    repo_config::RepoConfig,
    repository::{Repository, RepositoryIncarnation, git::GitHead, git::GitPackSpan},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, IsolationLevel, QueryFilter,
    QueryOrder, QueryResult, Statement, TransactionTrait, TryGetable,
};

const MAX_DEPENDENCY_RETRY_SECONDS: i64 = 30 * 60;

mod backfill;
mod reads;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencySnapshotFile {
    pub path: ScopePath,
    pub blob: SourceBlob,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyAnalysisClaim {
    pub incarnation: RepositoryIncarnation,
    pub repo_version: u64,
    pub git_head: GitHead,
    pub git_pack_spans: Vec<GitPackSpan>,
    pub analyzer_version: String,
    pub lease_generation: String,
    pub attempts: u32,
    pub repo_config: RepoConfig,
    pub files: Vec<DependencySnapshotFile>,
    pub reusable_analysis: Option<StoredDependencyAnalysis>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyCompletion {
    Completed,
    Stale,
}

pub(super) async fn enqueue_dependency_analysis_for_repository<C: ConnectionTrait>(
    conn: &C,
    repository: &Repository,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let Some(head) = &repository.git_head else {
        return Ok(());
    };
    enqueue_dependency_analysis_target(
        conn,
        &repository.incarnation(),
        repository.record.change_version,
        &head.head_oid,
        DEPENDENCY_ANALYZER_VERSION,
        now_unix,
    )
    .await
}

impl RepositoryStore {
    pub async fn enqueue_dependency_analysis(
        &self,
        repo_id: &str,
        analyzer_version: &str,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        validate_analyzer_version(analyzer_version)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let Some(row) = tx
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT repository.incarnation_id, repository.change_version, head.head_oid
                 FROM scope_repositories repository
                 JOIN scope_git_heads head ON head.repo_id = repository.id
                 WHERE repository.id = $1",
                [repo_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        };
        let incarnation =
            RepositoryIncarnation::new(repo_id, database_value::<String>(&row, "incarnation_id")?)
                .map_err(PostgresError::internal)?;
        enqueue_dependency_analysis_target(
            &tx,
            &incarnation,
            database_u64(&row, "change_version", "repository change version")?,
            &database_value::<String>(&row, "head_oid")?,
            analyzer_version,
            now_unix,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(true)
    }
}

impl JobStore {
    pub async fn claim_dependency_analysis(
        &self,
        worker_id: &str,
        analyzer_version: &str,
        now_unix: u64,
        lease_seconds: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<Option<DependencyAnalysisClaim>, PostgresError> {
        if worker_id.trim().is_empty() {
            return Err(PostgresError::internal_message(
                "dependency analysis worker identity is empty",
            ));
        }
        if lease_seconds == 0 {
            return Err(PostgresError::internal_message(
                "dependency analysis lease must be greater than zero",
            ));
        }
        validate_analyzer_version(analyzer_version)?;
        let now = dependency_time(now_unix)?;
        let lease_expires = now
            .checked_add(i64::try_from(lease_seconds).map_err(|_| {
                PostgresError::internal_message("dependency analysis lease exceeds database bigint")
            })?)
            .ok_or_else(|| {
                PostgresError::internal_message(
                    "dependency analysis lease expiry exceeds database bigint",
                )
            })?;
        let lease_generation =
            generate_id(generated_ids, GeneratedIdKind::DependencyAnalysisLease)?;
        let tx = self
            .db
            .begin_with_config(Some(IsolationLevel::RepeatableRead), None)
            .await
            .map_err(PostgresError::internal)?;
        let Some(job) = tx
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    UPDATE scope_dependency_analysis_jobs job
                    SET lease_generation = $3, lease_owner = $4,
                        lease_expires_at_unix = $2, updated_at_unix = GREATEST(updated_at_unix, $1)
                    FROM (
                        SELECT repo_id FROM scope_dependency_analysis_jobs
                        WHERE next_run_at_unix <= $1
                          AND (lease_expires_at_unix IS NULL OR lease_expires_at_unix <= $1)
                          AND analyzer_version = $5
                        ORDER BY next_run_at_unix, updated_at_unix, repo_id
                        FOR UPDATE SKIP LOCKED LIMIT 1
                    ) claimed
                    WHERE job.repo_id = claimed.repo_id
                    RETURNING job.*
                "#,
                [
                    now.into(),
                    lease_expires.into(),
                    lease_generation.clone().into(),
                    worker_id.into(),
                    analyzer_version.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let repo_id = database_value::<String>(&job, "repo_id")?;
        let repository = tx
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT incarnation_id, change_version, repo_config FROM scope_repositories WHERE id = $1",
                [repo_id.clone().into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("dependency job has no repository"))?;
        let head = super::entities::git_head::Entity::find_by_id(repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("dependency job has no Git head"))?
            .try_into_domain()?;
        let incarnation = RepositoryIncarnation::new(
            repo_id.clone(),
            database_value::<String>(&repository, "incarnation_id")?,
        )
        .map_err(PostgresError::internal)?;
        let repo_version =
            database_u64(&job, "repo_version", "dependency target repository version")?;
        if incarnation.incarnation_id() != database_value::<String>(&job, "incarnation_id")?
            || repo_version
                != database_u64(&repository, "change_version", "repository change version")?
            || head.head_oid != database_value::<String>(&job, "head_oid")?
        {
            let analyzer_version = database_value::<String>(&job, "analyzer_version")?;
            enqueue_dependency_analysis_target(
                &tx,
                &incarnation,
                database_u64(&repository, "change_version", "repository change version")?,
                &head.head_oid,
                &analyzer_version,
                now_unix,
            )
            .await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        }
        let rows = super::entities::live_file::Entity::find()
            .filter(super::entities::live_file::Column::RepoId.eq(repo_id.clone()))
            .order_by_asc(super::entities::live_file::Column::Path)
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        let files = rows
            .into_iter()
            .map(|row| {
                Ok(DependencySnapshotFile {
                    path: ScopePath::parse(row.path).map_err(PostgresError::internal)?,
                    blob: decode_json(row.content)?,
                })
            })
            .collect::<Result<Vec<_>, PostgresError>>()?;
        let analyzer_version = database_value::<String>(&job, "analyzer_version")?;
        let reusable_analysis = load_reusable_analysis(
            &tx,
            &repo_id,
            incarnation.incarnation_id(),
            &head.head_oid,
            &analyzer_version,
        )
        .await?;
        let claim = DependencyAnalysisClaim {
            incarnation,
            repo_version,
            git_pack_spans: load_git_pack_spans(&tx, &repo_id).await?,
            git_head: head,
            analyzer_version,
            lease_generation,
            attempts: database_u32(&job, "attempts", "dependency analysis attempts")?,
            repo_config: decode_json(database_value::<serde_json::Value>(
                &repository,
                "repo_config",
            )?)?,
            files,
            reusable_analysis,
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(claim))
    }

    pub async fn renew_dependency_analysis_claim(
        &self,
        claim: &DependencyAnalysisClaim,
        now_unix: u64,
        lease_seconds: u64,
    ) -> Result<bool, PostgresError> {
        if lease_seconds == 0 {
            return Err(PostgresError::internal_message(
                "dependency analysis lease must be greater than zero",
            ));
        }
        let now = dependency_time(now_unix)?;
        let expires = now
            .checked_add(i64::try_from(lease_seconds).map_err(|_| {
                PostgresError::internal_message("dependency analysis lease exceeds database bigint")
            })?)
            .ok_or_else(|| {
                PostgresError::internal_message(
                    "dependency analysis lease expiry exceeds database bigint",
                )
            })?;
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_dependency_analysis_jobs
                 SET lease_expires_at_unix = $3, updated_at_unix = GREATEST(updated_at_unix, $2)
                 WHERE repo_id = $1 AND lease_generation = $4 AND lease_expires_at_unix > $2",
                [
                    claim.incarnation.repository_id().into(),
                    now.into(),
                    expires.into(),
                    claim.lease_generation.clone().into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn complete_dependency_analysis_claim(
        &self,
        claim: &DependencyAnalysisClaim,
        output: AnalyzerOutput,
        now_unix: u64,
    ) -> Result<DependencyCompletion, PostgresError> {
        if output.analyzer_version != claim.analyzer_version {
            return Err(PostgresError::internal_message(
                "dependency analyzer output version does not match its claim",
            ));
        }
        let analysis = StoredDependencyAnalysis::from_output(&claim.git_head.head_oid, output)
            .map_err(PostgresError::internal)?;
        self.persist_dependency_analysis_claim(claim, analysis, now_unix)
            .await
    }

    pub async fn complete_reused_dependency_analysis_claim(
        &self,
        claim: &DependencyAnalysisClaim,
        now_unix: u64,
    ) -> Result<DependencyCompletion, PostgresError> {
        let analysis = claim.reusable_analysis.clone().ok_or_else(|| {
            PostgresError::internal_message("dependency claim has no reusable analysis")
        })?;
        self.persist_dependency_analysis_claim(claim, analysis, now_unix)
            .await
    }

    async fn persist_dependency_analysis_claim(
        &self,
        claim: &DependencyAnalysisClaim,
        analysis: StoredDependencyAnalysis,
        now_unix: u64,
    ) -> Result<DependencyCompletion, PostgresError> {
        if analysis.commit_oid != claim.git_head.head_oid
            || analysis.analyzer_version != claim.analyzer_version
        {
            return Err(PostgresError::internal_message(
                "dependency analysis identity does not match its claim",
            ));
        }
        let now = dependency_time(now_unix)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", claim.incarnation.repository_id()).await?;
        let Some(repository) = current_claim_repository(&tx, claim, now).await? else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(DependencyCompletion::Stale);
        };
        let config: RepoConfig = decode_json(database_value::<serde_json::Value>(
            &repository,
            "repo_config",
        )?)?;
        let report =
            evaluate_dependency_analysis(&analysis, &config).map_err(PostgresError::internal)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
                INSERT INTO scope_dependency_analyses (
                    repo_id, incarnation_id, head_oid, analyzer_version, analysis, completed_at_unix
                ) VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (repo_id) DO UPDATE SET
                    incarnation_id = EXCLUDED.incarnation_id, head_oid = EXCLUDED.head_oid,
                    analyzer_version = EXCLUDED.analyzer_version, analysis = EXCLUDED.analysis,
                    completed_at_unix = EXCLUDED.completed_at_unix
            "#,
            [
                claim.incarnation.repository_id().into(),
                claim.incarnation.incarnation_id().into(),
                claim.git_head.head_oid.clone().into(),
                claim.analyzer_version.clone().into(),
                serde_json::to_value(&analysis)
                    .map_err(PostgresError::internal)?
                    .into(),
                now.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
                INSERT INTO scope_dependency_reports (
                    repo_id, incarnation_id, repo_version, head_oid,
                    analyzer_version, report, completed_at_unix
                ) VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT (repo_id) DO UPDATE SET
                    incarnation_id = EXCLUDED.incarnation_id, repo_version = EXCLUDED.repo_version,
                    head_oid = EXCLUDED.head_oid, analyzer_version = EXCLUDED.analyzer_version,
                    report = EXCLUDED.report, completed_at_unix = EXCLUDED.completed_at_unix
            "#,
            [
                claim.incarnation.repository_id().into(),
                claim.incarnation.incarnation_id().into(),
                i64::try_from(claim.repo_version)
                    .map_err(|_| {
                        PostgresError::internal_message(
                            "dependency repository version exceeds database bigint",
                        )
                    })?
                    .into(),
                claim.git_head.head_oid.clone().into(),
                claim.analyzer_version.clone().into(),
                serde_json::to_value(&report)
                    .map_err(PostgresError::internal)?
                    .into(),
                now.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        let deleted = tx
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "DELETE FROM scope_dependency_analysis_jobs
             WHERE repo_id = $1 AND lease_generation = $2",
                [
                    claim.incarnation.repository_id().into(),
                    claim.lease_generation.clone().into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        if deleted.rows_affected() != 1 {
            tx.rollback().await.map_err(PostgresError::internal)?;
            return Ok(DependencyCompletion::Stale);
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(DependencyCompletion::Completed)
    }

    pub async fn fail_dependency_analysis_claim(
        &self,
        claim: &DependencyAnalysisClaim,
        error: &str,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let now = dependency_time(now_unix)?;
        let error = bounded_dependency_error(error);
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", claim.incarnation.repository_id()).await?;
        if current_claim_repository(&tx, claim, now).await?.is_none() {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        }
        let updated = tx
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                UPDATE scope_dependency_analysis_jobs
                SET lease_generation = NULL, lease_owner = NULL, lease_expires_at_unix = NULL,
                    attempts = attempts + 1,
                    next_run_at_unix = $3 + LEAST($5, 5 * (1::bigint << LEAST(attempts, 9))),
                    last_error = $4, updated_at_unix = GREATEST(updated_at_unix, $3)
                WHERE repo_id = $1 AND lease_generation = $2
            "#,
                [
                    claim.incarnation.repository_id().into(),
                    claim.lease_generation.clone().into(),
                    now.into(),
                    error.clone().into(),
                    MAX_DEPENDENCY_RETRY_SECONDS.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        if updated.rows_affected() != 1 {
            tx.rollback().await.map_err(PostgresError::internal)?;
            return Ok(false);
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(true)
    }
}

pub(super) async fn enqueue_dependency_analysis_target<C: ConnectionTrait>(
    conn: &C,
    incarnation: &RepositoryIncarnation,
    repo_version: u64,
    head_oid: &str,
    analyzer_version: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    validate_analyzer_version(analyzer_version)?;
    let version = i64::try_from(repo_version).map_err(|_| {
        PostgresError::internal_message("dependency repository version exceeds database bigint")
    })?;
    let now = dependency_time(now_unix)?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"
            INSERT INTO scope_dependency_analysis_jobs (
                repo_id, incarnation_id, repo_version, head_oid, analyzer_version,
                attempts, next_run_at_unix, lease_generation, lease_owner,
                lease_expires_at_unix, last_error, created_at_unix, updated_at_unix
            ) VALUES ($1, $2, $3, $4, $5, 0, $6, NULL, NULL, NULL, NULL, $6, $6)
            ON CONFLICT (repo_id) DO UPDATE SET
                incarnation_id = EXCLUDED.incarnation_id,
                repo_version = EXCLUDED.repo_version,
                head_oid = EXCLUDED.head_oid,
                analyzer_version = EXCLUDED.analyzer_version,
                attempts = 0,
                next_run_at_unix = EXCLUDED.next_run_at_unix,
                lease_generation = NULL,
                lease_owner = NULL,
                lease_expires_at_unix = NULL,
                last_error = NULL,
                updated_at_unix = GREATEST(scope_dependency_analysis_jobs.updated_at_unix, EXCLUDED.updated_at_unix)
            WHERE ROW(
                scope_dependency_analysis_jobs.incarnation_id,
                scope_dependency_analysis_jobs.repo_version,
                scope_dependency_analysis_jobs.head_oid,
                scope_dependency_analysis_jobs.analyzer_version
            ) IS DISTINCT FROM ROW(
                EXCLUDED.incarnation_id, EXCLUDED.repo_version,
                EXCLUDED.head_oid, EXCLUDED.analyzer_version
            )
        "#,
        [
            incarnation.repository_id().into(),
            incarnation.incarnation_id().into(),
            version.into(),
            head_oid.into(),
            analyzer_version.into(),
            now.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

async fn current_claim_repository<C: ConnectionTrait>(
    conn: &C,
    claim: &DependencyAnalysisClaim,
    now: i64,
) -> Result<Option<QueryResult>, PostgresError> {
    conn.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"
            SELECT repository.repo_config
            FROM scope_dependency_analysis_jobs job
            JOIN scope_repositories repository ON repository.id = job.repo_id
            JOIN scope_git_heads head ON head.repo_id = job.repo_id
            WHERE job.repo_id = $1 AND job.lease_generation = $2
              AND job.lease_expires_at_unix > $3
              AND job.incarnation_id = $4 AND job.repo_version = $5
              AND job.head_oid = $6 AND job.analyzer_version = $7
              AND repository.incarnation_id = job.incarnation_id
              AND repository.change_version = job.repo_version
              AND head.head_oid = job.head_oid
        "#,
        [
            claim.incarnation.repository_id().into(),
            claim.lease_generation.clone().into(),
            now.into(),
            claim.incarnation.incarnation_id().into(),
            i64::try_from(claim.repo_version)
                .map_err(|_| {
                    PostgresError::internal_message(
                        "dependency repository version exceeds database bigint",
                    )
                })?
                .into(),
            claim.git_head.head_oid.clone().into(),
            claim.analyzer_version.clone().into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)
}

async fn load_reusable_analysis<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    incarnation_id: &str,
    head_oid: &str,
    analyzer_version: &str,
) -> Result<Option<StoredDependencyAnalysis>, PostgresError> {
    let row = conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT analysis FROM scope_dependency_analyses
             WHERE repo_id = $1 AND incarnation_id = $2 AND head_oid = $3 AND analyzer_version = $4",
            [
                repo_id.into(),
                incarnation_id.into(),
                head_oid.into(),
                analyzer_version.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    row.map(|row| decode_json(database_value::<serde_json::Value>(&row, "analysis")?))
        .transpose()
}

fn validate_analyzer_version(version: &str) -> Result<(), PostgresError> {
    if version.trim().is_empty() || version.len() > 200 {
        return Err(PostgresError::internal_message(
            "dependency analyzer version must contain 1 to 200 bytes",
        ));
    }
    Ok(())
}

fn dependency_time(now_unix: u64) -> Result<i64, PostgresError> {
    i64::try_from(now_unix).map_err(|_| {
        PostgresError::internal_message("dependency analysis time exceeds database bigint")
    })
}

fn database_u64(row: &QueryResult, column: &str, label: &str) -> Result<u64, PostgresError> {
    u64::try_from(database_value::<i64>(row, column)?)
        .map_err(|_| PostgresError::internal_message(format!("{label} is negative")))
}

fn database_u32(row: &QueryResult, column: &str, label: &str) -> Result<u32, PostgresError> {
    u32::try_from(database_value::<i32>(row, column)?)
        .map_err(|_| PostgresError::internal_message(format!("{label} is invalid")))
}

fn database_value<T: TryGetable>(row: &QueryResult, column: &str) -> Result<T, PostgresError> {
    row.try_get::<T>("", column)
        .map_err(PostgresError::internal)
}

fn bounded_dependency_error(error: &str) -> String {
    let mut error = error.trim().chars().take(2_000).collect::<String>();
    if error.is_empty() {
        error = "dependency analysis failed without a diagnostic".into();
    }
    error
}
