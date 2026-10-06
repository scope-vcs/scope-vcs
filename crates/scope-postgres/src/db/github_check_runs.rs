use super::{
    RequestStore,
    entities::{decode_enum, encode_enum},
    integer_columns::{i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::requests::{GitHubCheckRun, RequestCheckEvaluation};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, TransactionTrait, Value,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubCheckCommit {
    pub repo_id: String,
    pub github_repository_id: u64,
    pub commit_oid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubCheckRefreshCandidate {
    pub commit: GitHubCheckCommit,
    pub request_id: String,
    pub head_oid: String,
}

#[derive(FromQueryResult)]
struct CandidateRow {
    repo_id: String,
    github_repository_id: i64,
    request_id: String,
    head_oid: String,
    tested_oid: String,
}

impl GitHubCheckCommit {
    fn key_values(&self) -> Result<Vec<Value>, PostgresError> {
        Ok(vec![
            self.repo_id.clone().into(),
            u64_to_i64(self.github_repository_id, "GitHub repository id")?.into(),
            self.commit_oid.clone().into(),
        ])
    }
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "start_github_check_read"))]
    pub async fn start_github_check_read(
        &self,
        commit: &GitHubCheckCommit,
    ) -> Result<u64, PostgresError> {
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_check_refreshes (repo_id, github_repository_id,
                    commit_oid, next_read_at_unix, started_reads, applied_read)
                 VALUES ($1, $2, $3, 0, 1, 0)
                 ON CONFLICT (repo_id, github_repository_id, commit_oid) DO UPDATE
                    SET started_reads = scope_github_check_refreshes.started_reads + 1
                 RETURNING started_reads",
                commit.key_values()?,
            ))
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("GitHub check read was not numbered"))?;
        i64_to_u64(
            row.try_get("", "started_reads")
                .map_err(PostgresError::internal)?,
            "GitHub check read",
        )
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "apply_github_check_read"))]
    pub async fn apply_github_check_read(
        &self,
        commit: &GitHubCheckCommit,
        read: u64,
        started_at_unix: u64,
        runs: &[GitHubCheckRun],
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let applied = tx
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT applied_read FROM scope_github_check_refreshes
                  WHERE repo_id = $1 AND github_repository_id = $2 AND commit_oid = $3
                  FOR UPDATE",
                commit.key_values()?,
            ))
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("GitHub check read was not started"))?
            .try_get::<i64>("", "applied_read")
            .map_err(PostgresError::internal)?;
        if i64_to_u64(applied, "GitHub check read")? >= read {
            return Ok(false);
        }
        let repository = u64_to_i64(commit.github_repository_id, "GitHub repository id")?;
        let started_at = u64_to_i64(started_at_unix, "GitHub check read time")?;
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_github_check_runs
              WHERE repo_id = $1 AND github_repository_id = $2 AND commit_oid = $3",
            commit.key_values()?,
        ))
        .await
        .map_err(PostgresError::internal)?;
        for run in runs
            .iter()
            .filter(|run| run.commit_oid == commit.commit_oid)
        {
            let values: Vec<Value> = vec![
                u64_to_i64(run.github_check_run_id, "GitHub check run id")?.into(),
                commit.repo_id.clone().into(),
                repository.into(),
                commit.commit_oid.clone().into(),
                run.name.clone().into(),
                encode_enum(run.status)?.into(),
                run.conclusion.map(encode_enum).transpose()?.into(),
                run.details_url.clone().into(),
                started_at.into(),
                optional_u64_to_i64(run.check_suite_id, "GitHub check suite id")?.into(),
            ];
            tx.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_check_runs (github_check_run_id, repo_id,
                    github_repository_id, commit_oid, name, status, conclusion, details_url,
                    updated_at_unix, check_suite_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT (github_check_run_id) DO UPDATE SET
                    repo_id = EXCLUDED.repo_id,
                    github_repository_id = EXCLUDED.github_repository_id,
                    commit_oid = EXCLUDED.commit_oid, name = EXCLUDED.name,
                    status = EXCLUDED.status, conclusion = EXCLUDED.conclusion,
                    details_url = EXCLUDED.details_url,
                    updated_at_unix = EXCLUDED.updated_at_unix,
                    check_suite_id = EXCLUDED.check_suite_id",
                values,
            ))
            .await
            .map_err(PostgresError::internal)?;
        }
        let mut values = commit.key_values()?;
        values.extend([
            u64_to_i64(read, "GitHub check read")?.into(),
            started_at.into(),
        ]);
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_github_check_refreshes
                SET applied_read = $4, applied_read_started_at_unix = $5
              WHERE repo_id = $1 AND github_repository_id = $2 AND commit_oid = $3",
            values,
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(true)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "settled_github_check_read_started_at"))]
    pub async fn settled_github_check_read_started_at(
        &self,
        commit: &GitHubCheckCommit,
    ) -> Result<Option<u64>, PostgresError> {
        let Some(row) = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT applied_read_started_at_unix FROM scope_github_check_refreshes
                  WHERE repo_id = $1 AND github_repository_id = $2 AND commit_oid = $3
                    AND started_reads = applied_read",
                commit.key_values()?,
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(None);
        };
        optional_i64_to_u64(
            row.try_get("", "applied_read_started_at_unix")
                .map_err(PostgresError::internal)?,
            "GitHub check read time",
        )
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "current_github_evaluations_testing"))]
    pub async fn current_github_evaluations_testing(
        &self,
        repo_id: &str,
        commit_oid: &str,
    ) -> Result<Vec<RequestCheckEvaluation>, PostgresError> {
        let heads = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                SELECT evaluation.request_id, evaluation.head_oid
                  FROM scope_request_check_evaluations evaluation
                  JOIN scope_requests request
                    ON request.id = evaluation.request_id
                   AND request.head_oid = evaluation.head_oid
                 WHERE request.repo_id = $1
                   AND evaluation.tested_oid = $2
                   AND evaluation.state = 'started'
                   AND evaluation.checks @? '$[*] ? (@.provider == "github")'
                   AND request.merged_at_unix IS NULL
                   AND request.closed_at_unix IS NULL
                "#,
                [repo_id.into(), commit_oid.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<String>("", "request_id")
                        .map_err(PostgresError::internal)?,
                    row.try_get::<String>("", "head_oid")
                        .map_err(PostgresError::internal)?,
                ))
            })
            .collect::<Result<Vec<_>, PostgresError>>()?;
        self.request_check_evaluations(&heads).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_commit_is_watched"))]
    pub async fn github_commit_is_watched(
        &self,
        repo_id: &str,
        commit_oid: &str,
    ) -> Result<bool, PostgresError> {
        if super::github_setup_checks::setup_check_watches(self.db.as_ref(), repo_id, commit_oid)
            .await?
        {
            return Ok(true);
        }
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

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_check_refresh_candidates"))]
    pub async fn github_check_refresh_candidates(
        &self,
        now_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubCheckRefreshCandidate>, PostgresError> {
        CandidateRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            SELECT request.repo_id, connection.github_repository_id, evaluation.request_id,
                   evaluation.head_oid, evaluation.tested_oid
              FROM scope_request_check_evaluations evaluation
              JOIN scope_requests request
                ON request.id = evaluation.request_id
               AND request.head_oid = evaluation.head_oid
              JOIN scope_github_connections connection
                ON connection.repo_id = request.repo_id
               AND connection.status = 'Connected'
              LEFT JOIN scope_github_check_refreshes refresh
                ON refresh.repo_id = request.repo_id
               AND refresh.github_repository_id = connection.github_repository_id
               AND refresh.commit_oid = evaluation.tested_oid
             WHERE evaluation.state = 'started'
               AND evaluation.checks @? '$[*] ? (@.provider == "github")'
               AND request.merged_at_unix IS NULL
               AND request.closed_at_unix IS NULL
               AND coalesce(refresh.next_read_at_unix, 0) <= $1
             ORDER BY coalesce(refresh.next_read_at_unix, 0), evaluation.request_id
             LIMIT $2
            "#,
            [
                u64_to_i64(now_unix, "GitHub check refresh time")?.into(),
                u64_to_i64(limit, "GitHub check refresh batch size")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| {
            Ok(GitHubCheckRefreshCandidate {
                commit: GitHubCheckCommit {
                    repo_id: row.repo_id,
                    github_repository_id: i64_to_u64(
                        row.github_repository_id,
                        "GitHub repository id",
                    )?,
                    commit_oid: row.tested_oid,
                },
                request_id: row.request_id,
                head_oid: row.head_oid,
            })
        })
        .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_github_check_refresh"))]
    pub async fn claim_github_check_refresh(
        &self,
        commit: &GitHubCheckCommit,
        now_unix: u64,
        next_read_at_unix: u64,
    ) -> Result<bool, PostgresError> {
        let mut values = commit.key_values()?;
        values.extend([
            u64_to_i64(next_read_at_unix, "GitHub check refresh time")?.into(),
            u64_to_i64(now_unix, "GitHub check refresh time")?.into(),
        ]);
        Ok(self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_check_refreshes (repo_id, github_repository_id,
                    commit_oid, next_read_at_unix, started_reads, applied_read)
                 VALUES ($1, $2, $3, $4, 0, 0)
                 ON CONFLICT (repo_id, github_repository_id, commit_oid) DO UPDATE
                    SET next_read_at_unix = EXCLUDED.next_read_at_unix
                  WHERE scope_github_check_refreshes.next_read_at_unix <= $5
                 RETURNING 1 AS claimed",
                values,
            ))
            .await
            .map_err(PostgresError::internal)?
            .is_some())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "schedule_github_check_refresh"))]
    pub async fn schedule_github_check_refresh(
        &self,
        commit: &GitHubCheckCommit,
        next_read_at_unix: u64,
    ) -> Result<(), PostgresError> {
        let mut values = commit.key_values()?;
        values.push(u64_to_i64(next_read_at_unix, "GitHub check refresh time")?.into());
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_github_check_refreshes SET next_read_at_unix = $4
                  WHERE repo_id = $1 AND github_repository_id = $2 AND commit_oid = $3",
                values,
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }
}

pub(super) async fn latest_github_check_runs<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    github_repository_id: u64,
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
               details_url, check_suite_id
          FROM scope_github_check_runs
         WHERE repo_id = $1
           AND github_repository_id = $2
           AND commit_oid IN (SELECT jsonb_array_elements_text($3::jsonb))
         ORDER BY commit_oid, name, github_check_run_id DESC
        "#,
        vec![
            repo_id.into(),
            u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
            serde_json::json!(commit_oids).into(),
        ],
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
            check_suite_id: optional_i64_to_u64(
                row.try_get("", "check_suite_id")
                    .map_err(PostgresError::internal)?,
                "GitHub check suite id",
            )?,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests;
