use super::super::{
    RequestStore, begin_metadata_read_snapshot,
    github_connections::repository_github_connection,
    integer_columns::{i64_to_u64, optional_i64_to_u64, u64_to_i64},
    request_checks::{evaluation_for_head, request_check_results},
    request_lifecycle_commands::{
        ExpectedGitHubCheckRead, ExpectedRequestCheckEvaluation, ExpectedRequestMergeEvidence,
    },
    request_revision_rows::latest_revision_for_request,
    request_rows::request_by_id,
};
use crate::error::PostgresError;
use scope_domain::requests::{Request, RequestCheckEvaluation, RequestCheckResults};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, Statement};

impl RequestStore {
    pub async fn request_merge_evidence(
        &self,
        request_id: &str,
        head_oid: &str,
        base_version: u64,
        review_base_oid: &str,
    ) -> Result<ExpectedRequestMergeEvidence, PostgresError> {
        let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let request = request_by_id(&tx, request_id)
            .await?
            .filter(|request| {
                request.head_oid == head_oid
                    && request.base_version == base_version
                    && request.base.review_base_oid() == review_base_oid
            })
            .ok_or_else(|| {
                PostgresError::conflict("request changed while confirming merge evidence")
            })?;
        let revision = latest_revision_for_request(&tx, request_id)
            .await?
            .filter(|revision| revision.new_head_oid == head_oid)
            .ok_or_else(|| PostgresError::conflict("request has no current revision"))?;
        let evaluation = evaluation_for_head(&tx, request_id, head_oid).await?;
        let expected = ExpectedRequestMergeEvidence {
            revision_id: revision.id,
            base_version: request.base_version,
            evaluation: match evaluation.as_ref() {
                Some(evaluation) => {
                    Some(expected_evaluation(&tx, &request, evaluation, false).await?)
                }
                None => None,
            },
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(expected)
    }
}

pub(super) struct LockedMergeChecks {
    pub(super) evaluation: Option<RequestCheckEvaluation>,
    pub(super) results: RequestCheckResults,
    valid_until_unix: Option<u64>,
}

impl LockedMergeChecks {
    pub(super) fn ensure_fresh(&self, now_unix: u64) -> Result<(), PostgresError> {
        if self
            .valid_until_unix
            .is_some_and(|expiry| expiry <= now_unix)
        {
            return Err(PostgresError::conflict(
                "request CI confirmation expired; retry merge",
            ));
        }
        Ok(())
    }
}

pub(super) async fn locked_merge_checks(
    tx: &DatabaseTransaction,
    request: &Request,
    expected: &ExpectedRequestMergeEvidence,
    now_unix: u64,
) -> Result<LockedMergeChecks, PostgresError> {
    let started = std::time::Instant::now();
    let revision = latest_revision_for_request(tx, &request.id)
        .await?
        .ok_or_else(|| PostgresError::conflict("request has no current revision"))?;
    if revision.id != expected.revision_id
        || revision.new_head_oid != request.head_oid
        || request.base_version != expected.base_version
    {
        return Err(stale_evidence());
    }
    tx.query_one_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT repo_id FROM scope_github_connections WHERE repo_id = $1 FOR UPDATE",
        [request.repo_id.clone().into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    let evaluation = evaluation_for_head(tx, &request.id, &request.head_oid).await?;
    let actual = match evaluation.as_ref() {
        Some(evaluation) => {
            if evaluation.revision_id != revision.id || !evaluation.matches_request(request) {
                return Err(stale_evidence());
            }
            Some(expected_evaluation(tx, request, evaluation, true).await?)
        }
        None => None,
    };
    if actual != expected.evaluation {
        return Err(stale_evidence());
    }
    let valid_until_unix = if let Some(evaluation) = &evaluation
        && evaluation.asks_github()
    {
        let destination = evaluation.github_repository_id.ok_or_else(stale_evidence)?;
        let read = read_github_sequence(
            tx,
            &request.repo_id,
            destination,
            &evaluation.tested_oid,
            false,
        )
        .await?
        .ok_or_else(stale_evidence)?;
        if read.started_reads != read.applied_read {
            return Err(PostgresError::conflict(
                "request CI confirmation changed; retry merge",
            ));
        }
        Some(
            read.started_at
                .ok_or_else(stale_evidence)?
                .saturating_add(scope_domain::requests::GITHUB_CHECK_MERGE_FRESHNESS_SECS),
        )
    } else {
        None
    };
    let results = request_check_results(tx, &request.repo_id, evaluation.as_ref()).await?;
    let checks = LockedMergeChecks {
        evaluation,
        results,
        valid_until_unix,
    };
    checks.ensure_fresh(now_unix.saturating_add(started.elapsed().as_secs()))?;
    Ok(checks)
}

async fn expected_evaluation(
    tx: &DatabaseTransaction,
    request: &Request,
    evaluation: &RequestCheckEvaluation,
    lock: bool,
) -> Result<ExpectedRequestCheckEvaluation, PostgresError> {
    let github = if evaluation.asks_github() {
        let connection = repository_github_connection(tx, &request.repo_id)
            .await?
            .filter(|connection| connection.is_connected())
            .ok_or_else(|| PostgresError::conflict("request CI is disconnected"))?;
        if evaluation.github_repository_id != Some(connection.github_repository_id) {
            return Err(stale_evidence());
        }
        let read = read_github_sequence(
            tx,
            &request.repo_id,
            connection.github_repository_id,
            &evaluation.tested_oid,
            lock,
        )
        .await?
        .filter(|read| read.started_reads == read.applied_read)
        .ok_or_else(|| {
            PostgresError::conflict("request CI confirmation is not settled; retry merge")
        })?;
        Some(ExpectedGitHubCheckRead {
            github_repository_id: connection.github_repository_id,
            applied_read: read.applied_read,
        })
    } else {
        None
    };
    Ok(ExpectedRequestCheckEvaluation {
        evaluation_id: evaluation.id.clone(),
        tested_oid: evaluation.tested_oid.clone(),
        github,
    })
}

struct GitHubReadSequence {
    started_reads: u64,
    applied_read: u64,
    started_at: Option<u64>,
}

async fn read_github_sequence(
    tx: &DatabaseTransaction,
    repo_id: &str,
    destination: u64,
    tested_oid: &str,
    lock: bool,
) -> Result<Option<GitHubReadSequence>, PostgresError> {
    let sql = if lock {
        "SELECT started_reads, applied_read, applied_read_started_at_unix FROM scope_github_check_refreshes WHERE repo_id = $1 AND github_repository_id = $2 AND commit_oid = $3 FOR UPDATE"
    } else {
        "SELECT started_reads, applied_read, applied_read_started_at_unix FROM scope_github_check_refreshes WHERE repo_id = $1 AND github_repository_id = $2 AND commit_oid = $3"
    };
    let row = tx
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            [
                repo_id.to_string().into(),
                u64_to_i64(destination, "GitHub repository id")?.into(),
                tested_oid.to_string().into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    row.map(|row| {
        Ok(GitHubReadSequence {
            started_reads: i64_to_u64(
                row.try_get("", "started_reads")
                    .map_err(PostgresError::internal)?,
                "GitHub check read",
            )?,
            applied_read: i64_to_u64(
                row.try_get("", "applied_read")
                    .map_err(PostgresError::internal)?,
                "GitHub check read",
            )?,
            started_at: optional_i64_to_u64(
                row.try_get("", "applied_read_started_at_unix")
                    .map_err(PostgresError::internal)?,
                "GitHub check read time",
            )?,
        })
    })
    .transpose()
}

fn stale_evidence() -> PostgresError {
    PostgresError::conflict("request merge evidence changed since preparation; retry merge")
}
