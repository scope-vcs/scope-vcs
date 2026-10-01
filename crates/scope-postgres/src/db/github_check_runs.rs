//! Check runs GitHub reported for commits in a repository. GitHub's API is the
//! source: each read replaces what Scope stored for the commit, and records
//! when it happened so the reconciler can pace its own reads.

use super::{
    RequestStore,
    entities::{decode_enum, encode_enum},
    integer_columns::{i64_to_u64, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::requests::GitHubCheckRun;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, TransactionTrait, Value,
};

/// A started GitHub evaluation of an open request's current head that the
/// reconciler may read results for.
#[derive(Clone, Debug, PartialEq, Eq, FromQueryResult)]
pub struct GitHubCheckRefreshCandidate {
    pub repo_id: String,
    pub request_id: String,
    pub head_oid: String,
    pub tested_oid: String,
}

impl RequestStore {
    /// Replaces the check runs stored for `commit_oid` with what GitHub reports.
    pub async fn replace_github_check_runs(
        &self,
        repo_id: &str,
        commit_oid: &str,
        runs: &[GitHubCheckRun],
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let now = u64_to_i64(now_unix, "GitHub check refresh time")?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_github_check_runs WHERE repo_id = $1 AND commit_oid = $2",
            [repo_id.into(), commit_oid.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
        for run in runs.iter().filter(|run| run.commit_oid == commit_oid) {
            let values: Vec<Value> = vec![
                u64_to_i64(run.github_check_run_id, "GitHub check run id")?.into(),
                repo_id.into(),
                commit_oid.into(),
                run.name.clone().into(),
                encode_enum(run.status)?.into(),
                run.conclusion.map(encode_enum).transpose()?.into(),
                run.details_url.clone().into(),
                now.into(),
            ];
            // A run id belongs to one GitHub repository, which may since have
            // been connected to another Scope repository.
            tx.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_check_runs (github_check_run_id, repo_id, commit_oid,
                    name, status, conclusion, details_url, updated_at_unix)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (github_check_run_id) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id, commit_oid = EXCLUDED.commit_oid,
                    name = EXCLUDED.name, status = EXCLUDED.status,
                    conclusion = EXCLUDED.conclusion, details_url = EXCLUDED.details_url,
                    updated_at_unix = EXCLUDED.updated_at_unix",
                values,
            ))
            .await
            .map_err(PostgresError::internal)?;
        }
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_github_check_refreshes (repo_id, commit_oid, refreshed_at_unix)
             VALUES ($1, $2, $3)
             ON CONFLICT (repo_id, commit_oid) DO UPDATE
                SET refreshed_at_unix = greatest(
                    scope_github_check_refreshes.refreshed_at_unix, EXCLUDED.refreshed_at_unix)",
            [repo_id.into(), commit_oid.into(), now.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)
    }

    /// Whether any request in the repository was evaluated against `commit_oid`.
    pub async fn github_commit_is_tested(
        &self,
        repo_id: &str,
        commit_oid: &str,
    ) -> Result<bool, PostgresError> {
        self.db
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT EXISTS (
                    SELECT 1
                      FROM scope_request_check_evaluations evaluation
                      JOIN scope_requests request ON request.id = evaluation.request_id
                     WHERE evaluation.tested_oid = $2 AND request.repo_id = $1
                 ) AS tested",
                [repo_id.into(), commit_oid.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("tested commit query returned no row"))?
            .try_get("", "tested")
            .map_err(PostgresError::internal)
    }

    /// Started GitHub evaluations of open requests' current heads in connected
    /// repositories whose results were last read at or before `stale_at_unix`,
    /// least recently read first.
    pub async fn github_check_refresh_candidates(
        &self,
        stale_at_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubCheckRefreshCandidate>, PostgresError> {
        GitHubCheckRefreshCandidate::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            SELECT request.repo_id, evaluation.request_id, evaluation.head_oid,
                   evaluation.tested_oid
              FROM scope_request_check_evaluations evaluation
              JOIN scope_requests request
                ON request.id = evaluation.request_id
               AND request.head_oid = evaluation.head_oid
              JOIN scope_github_connections connection
                ON connection.repo_id = request.repo_id
               AND connection.status = 'Connected'
              LEFT JOIN scope_github_check_refreshes refresh
                ON refresh.repo_id = request.repo_id
               AND refresh.commit_oid = evaluation.tested_oid
             WHERE evaluation.state = 'started'
               AND evaluation.checks @? '$[*] ? (@.provider == "github")'
               AND request.merged_at_unix IS NULL
               AND request.closed_at_unix IS NULL
               AND (refresh.refreshed_at_unix IS NULL OR refresh.refreshed_at_unix <= $1)
             ORDER BY coalesce(refresh.refreshed_at_unix, 0), evaluation.request_id
             LIMIT $2
            "#,
            [
                u64_to_i64(stale_at_unix, "GitHub check refresh time")?.into(),
                u64_to_i64(limit, "GitHub check refresh batch size")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)
    }

    /// Takes the next read of the commit's results for this process. `false`
    /// when another read happened after `stale_at_unix`.
    pub async fn claim_github_check_refresh(
        &self,
        repo_id: &str,
        commit_oid: &str,
        now_unix: u64,
        stale_at_unix: u64,
    ) -> Result<bool, PostgresError> {
        Ok(self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_check_refreshes (repo_id, commit_oid, refreshed_at_unix)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (repo_id, commit_oid) DO UPDATE
                    SET refreshed_at_unix = EXCLUDED.refreshed_at_unix
                  WHERE scope_github_check_refreshes.refreshed_at_unix <= $4
                 RETURNING 1 AS claimed",
                [
                    repo_id.into(),
                    commit_oid.into(),
                    u64_to_i64(now_unix, "GitHub check refresh time")?.into(),
                    u64_to_i64(stale_at_unix, "GitHub check refresh time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?
            .is_some())
    }
}

/// The latest run of each check name on each of the commits. The domain applies
/// the same rule; reading only the latest keeps re-runs from growing the read.
pub(super) async fn latest_github_check_runs<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    commit_oids: &[String],
) -> Result<Vec<GitHubCheckRun>, PostgresError> {
    if commit_oids.is_empty() {
        return Ok(Vec::new());
    }
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"
        SELECT DISTINCT ON (commit_oid, name)
               github_check_run_id, commit_oid, name, status, conclusion,
               details_url
          FROM scope_github_check_runs
         WHERE repo_id = $1
           AND commit_oid IN (SELECT jsonb_array_elements_text($2::jsonb))
         ORDER BY commit_oid, name, github_check_run_id DESC
        "#,
        vec![repo_id.into(), serde_json::json!(commit_oids).into()],
    ))
    .await
    .map_err(PostgresError::internal)?
    .into_iter()
    .map(|row| {
        let get_text = |column| {
            row.try_get::<String>("", column)
                .map_err(PostgresError::internal)
        };
        Ok(GitHubCheckRun {
            commit_oid: get_text("commit_oid")?,
            name: get_text("name")?,
            github_check_run_id: i64_to_u64(
                row.try_get("", "github_check_run_id")
                    .map_err(PostgresError::internal)?,
                "GitHub check run id",
            )?,
            status: decode_enum(get_text("status")?)?,
            conclusion: row
                .try_get::<Option<String>>("", "conclusion")
                .map_err(PostgresError::internal)?
                .map(decode_enum)
                .transpose()?,
            details_url: row
                .try_get("", "details_url")
                .map_err(PostgresError::internal)?,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests;
