use super::{RepositoryStore, integer_columns::u64_to_i64};
use crate::error::PostgresError;
use scope_domain::{
    requests::{GitFetchHead, GitFetchRequest, Request},
    views::ViewId,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

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
        let mut refs = std::collections::BTreeMap::<&str, Vec<GitFetchRequest>>::new();
        for request in requests {
            refs.entry(request.head_oid.as_str())
                .or_default()
                .push(GitFetchRequest {
                    id: request.id.clone(),
                    view: request.view.clone(),
                });
        }
        refs.entry(main_oid).or_default();
        let refs = refs
            .into_iter()
            .map(|(oid, requests)| serde_json::json!({ "head_oid": oid, "requests": if oid == main_oid { serde_json::Value::Null } else { serde_json::json!(requests) } }))
            .collect::<Vec<_>>();
        self.db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "INSERT INTO scope_git_fetch_advertisements
             (repo_id, visibility_fingerprint, view_id, reader_id, head_oid, requests, expires_at_unix)
             SELECT $1,$2,$3,$4,refs.head_oid,refs.requests,$6
             FROM jsonb_to_recordset($5) AS refs(head_oid text, requests jsonb)
             WHERE NOT EXISTS (
                 SELECT 1 FROM scope_git_fetch_advertisements saved
                 WHERE saved.repo_id = $1 AND saved.visibility_fingerprint = $2
                   AND saved.view_id = $3 AND saved.reader_id = $4
                   AND saved.head_oid = refs.head_oid AND saved.expires_at_unix >= $6
                   AND (saved.requests IS NULL
                        OR (refs.requests IS NOT NULL AND saved.requests @> refs.requests)))
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
                 expires_at_unix = EXCLUDED.expires_at_unix
             WHERE scope_git_fetch_advertisements.expires_at_unix < EXCLUDED.expires_at_unix
                 OR (scope_git_fetch_advertisements.requests IS NOT NULL
                     AND (EXCLUDED.requests IS NULL
                          OR NOT scope_git_fetch_advertisements.requests @> EXCLUDED.requests))",
            [reader.repo_id.into(), reader.visibility_fingerprint.into(), reader.view.as_str().into(),
             reader.user_id.unwrap_or_default().into(), serde_json::Value::Array(refs).into(),
             u64_to_i64(scope_domain::repository::git::git_fetch_advertisement_expiry(now), "Git advertisement expiry")?.into()])).await?;
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
            .query_all_raw(Statement::from_sql_and_values(
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
            ))
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
        store.db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
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
                .advertised_git_fetch_heads(
                    &reader,
                    &wants,
                    scope_domain::repository::git::git_fetch_advertisement_expiry(100) + 1
                )
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn an_unchanged_advertisement_is_rewritten_only_when_its_expiry_advances() {
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
        let saved = || async {
            let row = store
                .db
                .query_one_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT xmin::text AS version, expires_at_unix
                     FROM scope_git_fetch_advertisements WHERE repo_id = $1",
                    [repo.record.id.clone().into()],
                ))
                .await
                .unwrap()
                .unwrap();
            (
                row.try_get::<String>("", "version").unwrap(),
                row.try_get::<i64>("", "expires_at_unix").unwrap(),
            )
        };
        repositories
            .record_git_fetch_advertisement(&reader, "main", &[], 121)
            .await
            .unwrap();
        let first = saved().await;
        assert!(first.1 >= 421);
        repositories
            .record_git_fetch_advertisement(&reader, "main", &[], 125)
            .await
            .unwrap();
        assert_eq!(saved().await, first);
        repositories
            .record_git_fetch_advertisement(&reader, "main", &[], 160)
            .await
            .unwrap();
        let advanced = saved().await;
        assert_ne!(advanced.0, first.0);
        assert!(advanced.1 >= 460);
    }

    #[tokio::test]
    async fn a_request_head_is_rewritten_only_for_a_new_request_or_view() {
        use crate::db::github_pull_requests::tests::{REPO, submitted_full_view_request};
        let store = crate::db::requests::tests::postgres_store();
        let mut first = submitted_full_view_request(&store, "req_first").await;
        let mut second = submitted_full_view_request(&store, "req_second").await;
        first.head_oid = "f".repeat(40);
        second.head_oid = first.head_oid.clone();
        let repositories = store.repositories();
        let view = ViewId::private();
        let reader = GitFetchReader {
            repo_id: REPO,
            visibility_fingerprint: "visibility",
            view: &view,
            user_id: None,
        };
        let version = || async {
            store
                .db
                .query_one_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT xmin::text AS version FROM scope_git_fetch_advertisements
                     WHERE repo_id = $1 AND head_oid = $2",
                    [REPO.into(), "f".repeat(40).into()],
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get::<String>("", "version")
                .unwrap()
        };
        let advertised = || async {
            let heads = repositories
                .advertised_git_fetch_heads(&reader, &["f".repeat(40)], 121)
                .await
                .unwrap();
            let GitFetchHead::Requests(requests) = &heads[0].1 else {
                panic!("request head was saved as main")
            };
            requests
                .iter()
                .map(|request| (request.id.clone(), request.view.clone()))
                .collect::<std::collections::BTreeSet<_>>()
        };
        repositories
            .record_git_fetch_advertisement(&reader, "main", &[first.clone(), second.clone()], 121)
            .await
            .unwrap();
        let both = version().await;
        repositories
            .record_git_fetch_advertisement(&reader, "main", std::slice::from_ref(&second), 121)
            .await
            .unwrap();
        assert_eq!(version().await, both);

        second.view = ViewId::public();
        repositories
            .record_git_fetch_advertisement(&reader, "main", std::slice::from_ref(&second), 121)
            .await
            .unwrap();
        assert_ne!(version().await, both);
        assert_eq!(
            advertised().await,
            [
                ("req_first".to_string(), ViewId::private()),
                ("req_second".to_string(), ViewId::public()),
            ]
            .into_iter()
            .collect()
        );
    }
}
