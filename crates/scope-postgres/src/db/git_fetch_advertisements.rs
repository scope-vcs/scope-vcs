use super::{RepositoryStore, integer_columns::u64_to_i64};
use crate::error::PostgresError;
use scope_domain::{requests::Request, views::ViewId};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

const EXPIRED_ADVERTISEMENTS_PER_CALL: i64 = 500;

pub struct GitFetchReader<'a> {
    pub repo_id: &'a str,
    pub generation: &'a str,
    pub view: &'a ViewId,
    pub user_id: Option<&'a str>,
}

impl RepositoryStore {
    pub async fn record_git_fetch_advertisement(
        &self,
        reader: &GitFetchReader<'_>,
        main_oid: &str,
        requests: &[Request],
        now: u64,
    ) -> Result<(), PostgresError> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "DELETE FROM scope_git_fetch_advertisements WHERE ctid = ANY(ARRAY(
                     SELECT ctid FROM scope_git_fetch_advertisements
                     WHERE expires_at_unix < $1
                     LIMIT $2 FOR UPDATE SKIP LOCKED))",
                [
                    u64_to_i64(now, "Git advertisement time")?.into(),
                    EXPIRED_ADVERTISEMENTS_PER_CALL.into(),
                ],
            ))
            .await?;
        let mut refs = std::collections::BTreeMap::<&str, Option<Vec<&Request>>>::new();
        for request in requests {
            refs.entry(request.head_oid.as_str())
                .or_insert_with(|| Some(Vec::new()))
                .get_or_insert_default()
                .push(request);
        }
        refs.insert(main_oid, None);
        let refs = refs
            .into_iter()
            .map(|(oid, requests)| serde_json::json!({ "head_oid": oid, "requests": requests }))
            .collect::<Vec<_>>();
        self.db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "INSERT INTO scope_git_fetch_advertisements
             (repo_id, generation, view_id, reader_id, head_oid, requests, expires_at_unix)
             SELECT $1,$2,$3,$4,refs.head_oid,refs.requests,$6
             FROM jsonb_to_recordset($5) AS refs(head_oid text, requests jsonb)
             ON CONFLICT (repo_id, generation, view_id, reader_id, head_oid)
             DO UPDATE SET requests = CASE WHEN scope_git_fetch_advertisements.requests IS NULL
                 OR EXCLUDED.requests IS NULL THEN NULL ELSE EXCLUDED.requests END,
                 expires_at_unix = EXCLUDED.expires_at_unix",
            [reader.repo_id.into(), reader.generation.into(), reader.view.as_str().into(),
             reader.user_id.unwrap_or_default().into(), serde_json::Value::Array(refs).into(),
             u64_to_i64(now.saturating_add(scope_domain::repository::git::GIT_FETCH_ADVERTISEMENT_RETENTION_SECONDS), "Git advertisement expiry")?.into()])).await?;
        Ok(())
    }

    pub async fn advertised_git_fetch_heads(
        &self,
        reader: &GitFetchReader<'_>,
        wants: &[String],
        now: u64,
    ) -> Result<Vec<(String, Option<Vec<Request>>)>, PostgresError> {
        if wants.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT head_oid, requests FROM scope_git_fetch_advertisements
             WHERE repo_id = $1 AND generation = $2 AND view_id = $3 AND reader_id = $4
             AND head_oid = ANY($5) AND expires_at_unix >= $6",
                [
                    reader.repo_id.into(),
                    reader.generation.into(),
                    reader.view.as_str().into(),
                    reader.user_id.unwrap_or_default().into(),
                    wants.to_vec().into(),
                    u64_to_i64(now, "Git advertisement time")?.into(),
                ],
            ))
            .await?
            .into_iter()
            .map(|row| {
                let oid = row
                    .try_get("", "head_oid")
                    .map_err(PostgresError::internal)?;
                let requests: Option<serde_json::Value> = row
                    .try_get("", "requests")
                    .map_err(PostgresError::internal)?;
                Ok((
                    oid,
                    requests
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(PostgresError::internal)?,
                ))
            })
            .collect()
    }
}
