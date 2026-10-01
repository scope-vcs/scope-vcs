//! Check runs GitHub reported for commits in a repository.

use super::{entities::decode_enum, integer_columns::i64_to_u64};
use crate::error::PostgresError;
use scope_domain::requests::GitHubCheckRun;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

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
               details_url, started_at_unix
          FROM scope_github_check_runs
         WHERE repo_id = $1
           AND commit_oid IN (SELECT jsonb_array_elements_text($2::jsonb))
         ORDER BY commit_oid, name, started_at_unix DESC, github_check_run_id DESC
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
            started_at_unix: i64_to_u64(
                row.try_get("", "started_at_unix")
                    .map_err(PostgresError::internal)?,
                "GitHub check run start time",
            )?,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests;
