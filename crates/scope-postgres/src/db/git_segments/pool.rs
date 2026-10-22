use super::publish_git_segment;
use crate::db::integer_columns::{i32_to_u32, i64_to_u64, u32_to_i32, u64_to_i64};
use crate::error::PostgresError;
use scope_domain::{
    repository::git::{GitPoolPack, GitPoolPackStorage, GitSegmentRef},
    views::ViewId,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PooledRequestPack {
    pub id: String,
    pub request_id: String,
    pub head_oid: String,
    pub prerequisites: Vec<String>,
    pub request_view: ViewId,
    pub request_author_id: Option<String>,
    pub request_submitted: bool,
}

pub(in crate::db) async fn register_git_pool_pack<C>(
    conn: &C,
    repo_id: &str,
    pack: &GitPoolPack,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let (sealed, segment_id, sha256, plaintext_bytes, encoding_version): (
        Value,
        Value,
        Value,
        Value,
        Value,
    ) = match &pack.storage {
        GitPoolPackStorage::Inline { sealed } => (
            Some(sealed.clone()).into(),
            Option::<String>::None.into(),
            Option::<String>::None.into(),
            Option::<i64>::None.into(),
            Option::<i32>::None.into(),
        ),
        GitPoolPackStorage::Segment(segment) => {
            publish_git_segment(conn, repo_id, segment, now_unix).await?;
            (
                Option::<Vec<u8>>::None.into(),
                Some(segment.segment_id.clone()).into(),
                Some(segment.sha256.clone()).into(),
                Some(u64_to_i64(segment.plaintext_bytes, "Git pool pack size")?).into(),
                Some(u32_to_i32(
                    segment.encoding_version,
                    "Git pool pack encoding",
                )?)
                .into(),
            )
        }
    };
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_git_pool_packs (
             id, repo_id, request_id, head_oid, prerequisites, sealed_pack, segment_id,
             segment_sha256, segment_plaintext_bytes, segment_encoding_version, created_at_unix
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        [
            pack.id.clone().into(),
            repo_id.into(),
            pack.request_id.clone().into(),
            pack.head_oid.clone().into(),
            pack.prerequisites.clone().into(),
            sealed,
            segment_id,
            sha256,
            plaintext_bytes,
            encoding_version,
            u64_to_i64(now_unix, "Git pool pack registration time")?.into(),
        ],
    ))
    .await?;
    Ok(())
}

async fn git_pool_packs_for_requests<C>(
    conn: &C,
    repo_id: &str,
    request_ids: &[String],
) -> Result<Vec<PooledRequestPack>, PostgresError>
where
    C: ConnectionTrait,
{
    if request_ids.is_empty() {
        return Ok(Vec::new());
    }
    let read = PostgresError::internal;
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT packs.id, packs.request_id, packs.head_oid, packs.prerequisites,
                requests.audience AS request_view,
                requests.author_user_id AS request_author_id,
                requests.submitted_at_unix IS NOT NULL AS request_submitted
         FROM scope_git_pool_packs packs
         JOIN scope_requests requests ON requests.id = packs.request_id
         WHERE packs.repo_id = $1 AND packs.request_id = ANY($2)
         ORDER BY packs.created_at_unix, packs.id",
        [repo_id.into(), request_ids.to_vec().into()],
    ))
    .await?
    .iter()
    .map(|row| {
        Ok(PooledRequestPack {
            id: row.try_get("", "id").map_err(read)?,
            request_id: row.try_get("", "request_id").map_err(read)?,
            head_oid: row.try_get("", "head_oid").map_err(read)?,
            prerequisites: row.try_get("", "prerequisites").map_err(read)?,
            request_view: ViewId::parse(&row.try_get::<String>("", "request_view").map_err(read)?)
                .map_err(PostgresError::internal)?,
            request_author_id: row.try_get("", "request_author_id").map_err(read)?,
            request_submitted: row.try_get("", "request_submitted").map_err(read)?,
        })
    })
    .collect()
}

async fn git_pool_pack_storage<C>(
    conn: &C,
    repo_id: &str,
    pack_ids: &[String],
) -> Result<Vec<(String, GitPoolPackStorage)>, PostgresError>
where
    C: ConnectionTrait,
{
    if pack_ids.is_empty() {
        return Ok(Vec::new());
    }
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT id, sealed_pack, segment_id, segment_sha256, segment_plaintext_bytes,
                segment_encoding_version
         FROM scope_git_pool_packs
         WHERE repo_id = $1 AND id = ANY($2)",
        [repo_id.into(), pack_ids.to_vec().into()],
    ))
    .await?
    .iter()
    .map(|row| {
        let id = row.try_get("", "id").map_err(PostgresError::internal)?;
        Ok((id, pool_pack_storage(row)?))
    })
    .collect()
}

fn pool_pack_storage(row: &QueryResult) -> Result<GitPoolPackStorage, PostgresError> {
    let read = PostgresError::internal;
    Ok(
        match row
            .try_get::<Option<Vec<u8>>>("", "sealed_pack")
            .map_err(read)?
        {
            Some(sealed) => GitPoolPackStorage::Inline { sealed },
            None => GitPoolPackStorage::Segment(GitSegmentRef {
                segment_id: row.try_get("", "segment_id").map_err(read)?,
                sha256: row.try_get("", "segment_sha256").map_err(read)?,
                plaintext_bytes: i64_to_u64(
                    row.try_get("", "segment_plaintext_bytes").map_err(read)?,
                    "Git pool pack size",
                )?,
                encoding_version: i32_to_u32(
                    row.try_get("", "segment_encoding_version").map_err(read)?,
                    "Git pool pack encoding",
                )?,
            }),
        },
    )
}

impl crate::db::RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "git_pool_packs_for_requests"))]
    pub async fn git_pool_packs_for_requests(
        &self,
        repo_id: &str,
        request_ids: &[String],
    ) -> Result<Vec<PooledRequestPack>, PostgresError> {
        git_pool_packs_for_requests(self.db.as_ref(), repo_id, request_ids).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "git_pool_pack_storage"))]
    pub async fn git_pool_pack_storage(
        &self,
        repo_id: &str,
        pack_ids: &[String],
    ) -> Result<Vec<(String, GitPoolPackStorage)>, PostgresError> {
        git_pool_pack_storage(self.db.as_ref(), repo_id, pack_ids).await
    }
}

pub(in crate::db) async fn retire_request_pool_packs<C>(
    conn: &C,
    request_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let segment_ids = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT segment_id FROM scope_git_pool_packs
             WHERE request_id = $1 AND segment_id IS NOT NULL ORDER BY segment_id",
            [request_id.into()],
        ))
        .await?
        .into_iter()
        .map(|row| row.try_get::<String>("", "segment_id"))
        .collect::<Result<Vec<_>, _>>()
        .map_err(PostgresError::internal)?;
    for segment_id in segment_ids {
        super::retire_git_segment(conn, &segment_id, now_unix).await?;
    }
    Ok(())
}
