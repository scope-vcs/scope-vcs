use super::{
    database_u64, database_value, enqueue_dependency_analysis_target, validate_analyzer_version,
};
use crate::{db::JobStore, error::PostgresError};
use scope_domain::repository::RepositoryIncarnation;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

impl JobStore {
    pub async fn enqueue_dependency_analysis_backfill(
        &self,
        analyzer_version: &str,
        now_unix: u64,
        limit: usize,
    ) -> Result<u64, PostgresError> {
        validate_analyzer_version(analyzer_version)?;
        if limit == 0 {
            return Ok(0);
        }
        let limit = i64::try_from(limit).map_err(|_| {
            PostgresError::internal_message("dependency backfill limit exceeds database bigint")
        })?;
        let candidates = self
            .db
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    SELECT repository.id
                    FROM scope_repositories repository
                    JOIN scope_git_heads head ON head.repo_id = repository.id
                    LEFT JOIN scope_dependency_reports report ON report.repo_id = repository.id
                    LEFT JOIN scope_dependency_analysis_jobs job ON job.repo_id = repository.id
                    WHERE (
                        report.repo_id IS NULL OR ROW(
                            report.incarnation_id, report.repo_version,
                            report.head_oid, report.analyzer_version
                        ) IS DISTINCT FROM ROW(
                            repository.incarnation_id, repository.change_version,
                            head.head_oid, $1
                        )
                    ) AND (
                        job.repo_id IS NULL OR ROW(
                            job.incarnation_id, job.repo_version,
                            job.head_oid, job.analyzer_version
                        ) IS DISTINCT FROM ROW(
                            repository.incarnation_id, repository.change_version,
                            head.head_oid, $1
                        )
                    )
                    ORDER BY repository.id
                    LIMIT $2
                "#,
                [analyzer_version.into(), limit.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        let mut enqueued = 0_u64;
        for candidate in candidates {
            let repo_id = database_value::<String>(&candidate, "id")?;
            let tx = self.db.begin().await.map_err(PostgresError::internal)?;
            super::acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
            let current = tx
                .query_one(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    r#"
                        SELECT repository.incarnation_id, repository.change_version,
                            head.head_oid,
                            COALESCE(ROW(
                                report.incarnation_id, report.repo_version,
                                report.head_oid, report.analyzer_version
                            ) IS NOT DISTINCT FROM ROW(
                                repository.incarnation_id, repository.change_version,
                                head.head_oid, $2
                            ), FALSE) AS report_current,
                            COALESCE(ROW(
                                job.incarnation_id, job.repo_version,
                                job.head_oid, job.analyzer_version
                            ) IS NOT DISTINCT FROM ROW(
                                repository.incarnation_id, repository.change_version,
                                head.head_oid, $2
                            ), FALSE) AS job_current
                        FROM scope_repositories repository
                        JOIN scope_git_heads head ON head.repo_id = repository.id
                        LEFT JOIN scope_dependency_reports report ON report.repo_id = repository.id
                        LEFT JOIN scope_dependency_analysis_jobs job ON job.repo_id = repository.id
                        WHERE repository.id = $1
                    "#,
                    [repo_id.clone().into(), analyzer_version.into()],
                ))
                .await
                .map_err(PostgresError::internal)?;
            let Some(current) = current else {
                tx.commit().await.map_err(PostgresError::internal)?;
                continue;
            };
            if database_value::<bool>(&current, "report_current")?
                || database_value::<bool>(&current, "job_current")?
            {
                tx.commit().await.map_err(PostgresError::internal)?;
                continue;
            }
            let incarnation = RepositoryIncarnation::new(
                &repo_id,
                database_value::<String>(&current, "incarnation_id")?,
            )
            .map_err(PostgresError::internal)?;
            enqueue_dependency_analysis_target(
                &tx,
                &incarnation,
                database_u64(&current, "change_version", "repository change version")?,
                &database_value::<String>(&current, "head_oid")?,
                analyzer_version,
                now_unix,
            )
            .await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            enqueued += 1;
        }
        Ok(enqueued)
    }
}
