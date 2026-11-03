use super::data_scope::{DataScope, SystemReason};
use super::{RepositoryStore, integer_columns::u64_to_i64};
use crate::error::PostgresError;
use scope_domain::{
    requests::{GitFetchHead, Request},
    views::ViewId,
};
use sea_orm::{DatabaseBackend, Statement};

const EXPIRED_ADVERTISEMENTS_PER_CALL: i64 = 500;

pub struct GitFetchReader<'a> {
    pub repo_id: &'a str,
    pub visibility_fingerprint: &'a str,
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
            .execute(
                DataScope::System(SystemReason::GitFetchAdvertisementRetention),
                Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "DELETE FROM scope_git_fetch_advertisements WHERE ctid = ANY(ARRAY(
                     SELECT ctid FROM scope_git_fetch_advertisements
                     WHERE expires_at_unix < $1
                     LIMIT $2 FOR UPDATE SKIP LOCKED))",
                    [
                        u64_to_i64(now, "Git advertisement time")?.into(),
                        EXPIRED_ADVERTISEMENTS_PER_CALL.into(),
                    ],
                ),
            )
            .await?;
        let mut refs = std::collections::BTreeMap::<&str, Vec<&Request>>::new();
        for request in requests {
            refs.entry(request.head_oid.as_str())
                .or_default()
                .push(request);
        }
        refs.entry(main_oid).or_default();
        let refs = refs
            .into_iter()
            .map(|(oid, requests)| serde_json::json!({ "head_oid": oid, "requests": if oid == main_oid { serde_json::Value::Null } else { serde_json::json!(requests) } }))
            .collect::<Vec<_>>();
        self.db.execute(DataScope::Repository(reader.repo_id), Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "INSERT INTO scope_git_fetch_advertisements
             (repo_id, visibility_fingerprint, view_id, reader_id, head_oid, requests, expires_at_unix)
             SELECT $1,$2,$3,$4,refs.head_oid,refs.requests,$6
             FROM jsonb_to_recordset($5) AS refs(head_oid text, requests jsonb)
             ON CONFLICT (repo_id, visibility_fingerprint, view_id, reader_id, head_oid)
             DO UPDATE SET requests = CASE WHEN scope_git_fetch_advertisements.requests IS NULL
                 OR EXCLUDED.requests IS NULL THEN NULL ELSE (
                     SELECT jsonb_agg(merged.request) FROM (
                         SELECT DISTINCT ON (saved.request->>'id') saved.request
                         FROM jsonb_array_elements(
                             scope_git_fetch_advertisements.requests || EXCLUDED.requests
                         ) WITH ORDINALITY AS saved(request, position)
                         ORDER BY saved.request->>'id', saved.position DESC
                     ) merged
                 ) END,
                 expires_at_unix = EXCLUDED.expires_at_unix",
            [reader.repo_id.into(), reader.visibility_fingerprint.into(), reader.view.as_str().into(),
             reader.user_id.unwrap_or_default().into(), serde_json::Value::Array(refs).into(),
             u64_to_i64(now.saturating_add(scope_domain::repository::git::GIT_FETCH_ADVERTISEMENT_RETENTION_SECONDS), "Git advertisement expiry")?.into()])).await?;
        Ok(())
    }

    pub async fn advertised_git_fetch_heads(
        &self,
        reader: &GitFetchReader<'_>,
        wants: &[String],
        now: u64,
    ) -> Result<Vec<(String, GitFetchHead)>, PostgresError> {
        if wants.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .query_all(
                DataScope::Repository(reader.repo_id),
                Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT head_oid, requests FROM scope_git_fetch_advertisements
             WHERE repo_id = $1 AND visibility_fingerprint = $2 AND view_id = $3 AND reader_id = $4
             AND head_oid = ANY($5) AND expires_at_unix >= $6",
                    [
                        reader.repo_id.into(),
                        reader.visibility_fingerprint.into(),
                        reader.view.as_str().into(),
                        reader.user_id.unwrap_or_default().into(),
                        wants.to_vec().into(),
                        u64_to_i64(now, "Git advertisement time")?.into(),
                    ],
                ),
            )
            .await?
            .into_iter()
            .filter_map(|row| decode_head(row).transpose())
            .collect()
    }
}

fn decode_head(row: sea_orm::QueryResult) -> Result<Option<(String, GitFetchHead)>, PostgresError> {
    let oid = row
        .try_get("", "head_oid")
        .map_err(PostgresError::internal)?;
    let requests: Option<serde_json::Value> = row
        .try_get("", "requests")
        .map_err(PostgresError::internal)?;
    let head = match requests {
        None | Some(serde_json::Value::Null) => GitFetchHead::Main,
        Some(value) => match serde_json::from_value(value) {
            Ok(requests) => GitFetchHead::Requests(requests),
            Err(_) => return Ok(None),
        },
    };
    Ok(Some((oid, head)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CatalogFixture, test_support::fixtures};

    #[tokio::test]
    async fn advertisements_expire_and_skip_undecodable_request_heads() {
        let owner = fixtures::user("owner", "owner");
        let repo = fixtures::repository(&owner, "repo", ViewId::public());
        let mut catalog = CatalogFixture::default();
        catalog.users.insert(owner.id.clone(), owner);
        catalog
            .repositories
            .insert(repo.record.id.clone(), repo.clone());
        let store = fixtures::seeded_store(catalog);
        let repositories = store.repositories();
        let view = ViewId::public();
        let reader = GitFetchReader {
            repo_id: &repo.record.id,
            visibility_fingerprint: "visibility",
            view: &view,
            user_id: None,
        };
        repositories
            .record_git_fetch_advertisement(&reader, "main", &[], 100)
            .await
            .unwrap();
        store.db.execute(DataScope::Repository(&repo.record.id), Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "INSERT INTO scope_git_fetch_advertisements
             (repo_id, visibility_fingerprint, view_id, reader_id, head_oid, requests, expires_at_unix)
             VALUES ($1, 'visibility', 'public', '', 'future-request', '[{\"id\":\"future-request\"}]', 400)",
            [repo.record.id.clone().into()],
        )).await.unwrap();
        let wants = vec!["main".to_string(), "future-request".to_string()];
        let heads = repositories
            .advertised_git_fetch_heads(&reader, &wants, 100)
            .await
            .unwrap();
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].0, "main");
        assert!(matches!(heads[0].1, GitFetchHead::Main));
        assert!(
            repositories
                .advertised_git_fetch_heads(&reader, &wants, 401)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
