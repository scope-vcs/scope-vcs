use super::publish_git_segment;
use crate::db::integer_columns::{i32_to_u32, i64_to_u64, u32_to_i32, u64_to_i64};
use crate::error::PostgresError;
use scope_domain::{
    repository::git::{
        GitPoolPack, GitPoolPackOwner, GitPoolPackStorage, GitSegmentRef, RequestPackScope,
    },
    views::ViewId,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PooledRequest {
    pub id: String,
    pub view: ViewId,
    pub author_id: Option<String>,
    pub submitted: bool,
    pub invitees: Vec<String>,
    pub has_full_scope_packs: bool,
    pub has_own_view_packs: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PooledRequestPack {
    pub id: String,
    pub request_id: String,
    pub head_oid: String,
    pub prerequisites: Vec<String>,
    pub scope: RequestPackScope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedMainPoolPack {
    pub id: String,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub head_oid: String,
    pub prerequisites: Vec<String>,
}

impl ListedMainPoolPack {
    pub fn with_storage(self, storage: GitPoolPackStorage) -> GitPoolPack {
        GitPoolPack {
            id: self.id,
            owner: GitPoolPackOwner::Main {
                first_sequence: self.first_sequence,
                last_sequence: self.last_sequence,
            },
            head_oid: self.head_oid,
            prerequisites: self.prerequisites,
            storage,
        }
    }
}

pub(in crate::db) async fn register_request_pool_pack<C: ConnectionTrait>(
    conn: &C,
    request: &scope_domain::requests::Request,
    pack: &GitPoolPack,
    now_unix: u64,
) -> Result<(), PostgresError> {
    if !pack.belongs_to_request(request) {
        return Err(PostgresError::internal_message(
            "Git pool pack does not belong to the recorded revision",
        ));
    }
    register_git_pool_pack(conn, &request.repo_id, pack, now_unix).await
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
    let (request_id, first_sequence, last_sequence, view_id): (Value, Value, Value, Value) =
        match &pack.owner {
            GitPoolPackOwner::Request { request_id, scope } => (
                Some(request_id.clone()).into(),
                Option::<i64>::None.into(),
                Option::<i64>::None.into(),
                scope.view().map(|view| view.as_str().to_string()).into(),
            ),
            GitPoolPackOwner::Main {
                first_sequence,
                last_sequence,
            } => (
                Option::<String>::None.into(),
                Some(u64_to_i64(*first_sequence, "Git main pack first sequence")?).into(),
                Some(u64_to_i64(*last_sequence, "Git main pack last sequence")?).into(),
                Option::<String>::None.into(),
            ),
        };
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_git_pool_packs (
             id, repo_id, request_id, head_oid, prerequisites, sealed_pack, segment_id,
             segment_sha256, segment_plaintext_bytes, segment_encoding_version, created_at_unix,
             first_sequence, last_sequence, view_id
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        [
            pack.id.clone().into(),
            repo_id.into(),
            request_id,
            pack.head_oid.clone().into(),
            pack.prerequisites.clone().into(),
            sealed,
            segment_id,
            sha256,
            plaintext_bytes,
            encoding_version,
            u64_to_i64(now_unix, "Git pool pack registration time")?.into(),
            first_sequence,
            last_sequence,
            view_id,
        ],
    ))
    .await?;
    Ok(())
}

async fn git_pool_request_lineage<C>(
    conn: &C,
    repo_id: &str,
    request_ids: &[String],
) -> Result<Vec<PooledRequest>, PostgresError>
where
    C: ConnectionTrait,
{
    if request_ids.is_empty() {
        return Ok(Vec::new());
    }
    let read = PostgresError::internal;
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "WITH RECURSIVE lineage(id) AS (
             SELECT unnest($2::text[]) UNION
             SELECT unnest(ARRAY[r.parent_request_id] || ARRAY(
                 SELECT parent_request_id FROM scope_request_revisions
                 WHERE request_id = r.id AND parent_request_id IS NOT NULL))
             FROM scope_requests r JOIN lineage l ON r.id = l.id WHERE r.repo_id = $1
         ) SELECT requests.id, requests.audience, requests.author_user_id,
                requests.submitted_at_unix IS NOT NULL AS submitted,
                ARRAY(SELECT user_id FROM scope_request_invitees WHERE request_id = requests.id) AS invitees,
                EXISTS(SELECT 1 FROM scope_git_pool_packs packs
                       WHERE packs.repo_id = $1 AND packs.request_id = requests.id
                         AND packs.view_id IS NULL) AS has_full_scope_packs,
                EXISTS(SELECT 1 FROM scope_git_pool_packs packs
                       WHERE packs.repo_id = $1 AND packs.request_id = requests.id
                         AND packs.view_id = requests.audience) AS has_own_view_packs
         FROM scope_requests requests
         WHERE requests.repo_id = $1 AND requests.id IN (SELECT id FROM lineage)
         ORDER BY requests.id",
        [repo_id.into(), request_ids.to_vec().into()],
    ))
    .await?
    .iter()
    .map(|row| {
        Ok(PooledRequest {
            id: row.try_get("", "id").map_err(read)?,
            view: ViewId::parse(&row.try_get::<String>("", "audience").map_err(read)?)
                .map_err(PostgresError::internal)?,
            author_id: row.try_get("", "author_user_id").map_err(read)?,
            submitted: row.try_get("", "submitted").map_err(read)?,
            invitees: row.try_get("", "invitees").map_err(read)?,
            has_full_scope_packs: row.try_get("", "has_full_scope_packs").map_err(read)?,
            has_own_view_packs: row.try_get("", "has_own_view_packs").map_err(read)?,
        })
    })
    .collect()
}

async fn git_pool_packs_behind<C>(
    conn: &C,
    repo_id: &str,
    request_ids: &[String],
    heads: &[String],
    depth: u32,
) -> Result<Vec<PooledRequestPack>, PostgresError>
where
    C: ConnectionTrait,
{
    if request_ids.is_empty() || heads.is_empty() || depth == 0 {
        return Ok(Vec::new());
    }
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "WITH RECURSIVE walk(id, request_id, head_oid, prerequisites, view_id, depth) AS (
             SELECT id, request_id, head_oid, prerequisites, view_id, 1
             FROM scope_git_pool_packs
             WHERE repo_id = $1 AND request_id = ANY($2) AND head_oid = ANY($3)
             UNION
             SELECT packs.id, packs.request_id, packs.head_oid, packs.prerequisites,
                    packs.view_id, walk.depth + 1
             FROM walk JOIN scope_git_pool_packs packs ON packs.head_oid = ANY(walk.prerequisites)
             WHERE walk.depth < $4 AND packs.repo_id = $1 AND packs.request_id = ANY($2)
         ) SELECT DISTINCT ON (id) id, request_id, head_oid, prerequisites, view_id
         FROM walk ORDER BY id, depth",
        [
            repo_id.into(),
            request_ids.to_vec().into(),
            heads.to_vec().into(),
            u32_to_i32(depth, "Git pool pack walk depth")?.into(),
        ],
    ))
    .await?
    .iter()
    .map(pooled_request_pack)
    .collect()
}

async fn request_pool_packs<C>(
    conn: &C,
    repo_id: &str,
    request_id: &str,
) -> Result<Vec<PooledRequestPack>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT id, request_id, head_oid, prerequisites, view_id
         FROM scope_git_pool_packs
         WHERE repo_id = $1 AND request_id = $2
         ORDER BY created_at_unix, id",
        [repo_id.into(), request_id.into()],
    ))
    .await?
    .iter()
    .map(pooled_request_pack)
    .collect()
}

fn pooled_request_pack(row: &QueryResult) -> Result<PooledRequestPack, PostgresError> {
    let read = PostgresError::internal;
    Ok(PooledRequestPack {
        id: row.try_get("", "id").map_err(read)?,
        request_id: row.try_get("", "request_id").map_err(read)?,
        scope: match row.try_get::<Option<String>>("", "view_id").map_err(read)? {
            Some(view) => {
                RequestPackScope::View(ViewId::parse(&view).map_err(PostgresError::internal)?)
            }
            None => RequestPackScope::Full,
        },
        head_oid: row.try_get("", "head_oid").map_err(read)?,
        prerequisites: row.try_get("", "prerequisites").map_err(read)?,
    })
}

async fn main_pool_pack_listing<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<ListedMainPoolPack>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT id, first_sequence, last_sequence, head_oid, prerequisites
         FROM scope_git_pool_packs
         WHERE repo_id = $1 AND request_id IS NULL
         ORDER BY first_sequence",
        [repo_id.into()],
    ))
    .await?
    .iter()
    .map(listed_main_pool_pack)
    .collect()
}

pub(in crate::db) async fn load_main_pool_packs<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<GitPoolPack>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT id, first_sequence, last_sequence, head_oid, prerequisites, sealed_pack,
                segment_id, segment_sha256, segment_plaintext_bytes, segment_encoding_version
         FROM scope_git_pool_packs
         WHERE repo_id = $1 AND request_id IS NULL
         ORDER BY first_sequence",
        [repo_id.into()],
    ))
    .await?
    .iter()
    .map(|row| Ok(listed_main_pool_pack(row)?.with_storage(pool_pack_storage(row)?)))
    .collect()
}

fn listed_main_pool_pack(row: &QueryResult) -> Result<ListedMainPoolPack, PostgresError> {
    let read = PostgresError::internal;
    Ok(ListedMainPoolPack {
        id: row.try_get("", "id").map_err(read)?,
        first_sequence: i64_to_u64(
            row.try_get("", "first_sequence").map_err(read)?,
            "Git main pack first sequence",
        )?,
        last_sequence: i64_to_u64(
            row.try_get("", "last_sequence").map_err(read)?,
            "Git main pack last sequence",
        )?,
        head_oid: row.try_get("", "head_oid").map_err(read)?,
        prerequisites: row.try_get("", "prerequisites").map_err(read)?,
    })
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
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "git_pool_request_lineage"))]
    pub async fn git_pool_request_lineage(
        &self,
        repo_id: &str,
        request_ids: &[String],
    ) -> Result<Vec<PooledRequest>, PostgresError> {
        git_pool_request_lineage(self.db.as_ref(), repo_id, request_ids).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "git_pool_packs_behind"))]
    pub async fn git_pool_packs_behind(
        &self,
        repo_id: &str,
        request_ids: &[String],
        heads: &[String],
        depth: u32,
    ) -> Result<Vec<PooledRequestPack>, PostgresError> {
        git_pool_packs_behind(self.db.as_ref(), repo_id, request_ids, heads, depth).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_pool_packs"))]
    pub async fn request_pool_packs(
        &self,
        repo_id: &str,
        request_id: &str,
    ) -> Result<Vec<PooledRequestPack>, PostgresError> {
        request_pool_packs(self.db.as_ref(), repo_id, request_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "main_pool_packs"))]
    pub async fn main_pool_packs(
        &self,
        repo_id: &str,
    ) -> Result<Vec<ListedMainPoolPack>, PostgresError> {
        main_pool_pack_listing(self.db.as_ref(), repo_id).await
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
