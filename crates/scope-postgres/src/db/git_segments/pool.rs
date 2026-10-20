use super::publish_git_segment;
use crate::db::integer_columns::{i32_to_u32, i64_to_u64, u32_to_i32, u64_to_i64};
use crate::error::PostgresError;
use scope_domain::repository::git::{GitPoolPack, GitSegmentRef};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitPoolPacks {
    pub packs: Vec<GitPoolPack>,
    pub watermark: i64,
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
    publish_git_segment(conn, repo_id, &pack.segment, now_unix).await?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_git_pool_packs (
             segment_id, repo_id, request_id, head_oid, prerequisites,
             sha256, plaintext_bytes, encoding_version, created_at_unix
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        [
            pack.segment.segment_id.clone().into(),
            repo_id.into(),
            pack.request_id.clone().into(),
            pack.head_oid.clone().into(),
            pack.prerequisites.clone().into(),
            pack.segment.sha256.clone().into(),
            u64_to_i64(pack.segment.plaintext_bytes, "Git pool pack size")?.into(),
            u32_to_i32(pack.segment.encoding_version, "Git pool pack encoding")?.into(),
            u64_to_i64(now_unix, "Git pool pack registration time")?.into(),
        ],
    ))
    .await?;
    Ok(())
}

async fn git_pool_packs_after<C>(
    conn: &C,
    repo_id: &str,
    watermark: Option<i64>,
) -> Result<GitPoolPacks, PostgresError>
where
    C: ConnectionTrait,
{
    let rows = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT (pg_snapshot_xmin(pg_current_snapshot())::text)::bigint AS next_watermark,
                    packs.segment_id, packs.request_id, packs.head_oid, packs.prerequisites,
                    packs.sha256, packs.plaintext_bytes, packs.encoding_version
             FROM (SELECT 1) AS snapshot
             LEFT JOIN scope_git_pool_packs packs
               ON packs.repo_id = $1 AND packs.transaction_id >= $2
             ORDER BY packs.transaction_id, packs.segment_id",
            [repo_id.into(), watermark.unwrap_or(i64::MAX).into()],
        ))
        .await?;
    let watermark = rows
        .first()
        .ok_or_else(|| PostgresError::internal_message("snapshot watermark is missing"))?
        .try_get::<i64>("", "next_watermark")
        .map_err(PostgresError::internal)?;
    let packs = rows
        .iter()
        .filter(|row| {
            row.try_get::<Option<String>>("", "segment_id")
                .is_ok_and(|segment_id| segment_id.is_some())
        })
        .map(|row| {
            let read = PostgresError::internal;
            Ok(GitPoolPack {
                segment: GitSegmentRef {
                    segment_id: row.try_get("", "segment_id").map_err(read)?,
                    sha256: row.try_get("", "sha256").map_err(read)?,
                    plaintext_bytes: i64_to_u64(
                        row.try_get("", "plaintext_bytes").map_err(read)?,
                        "Git pool pack size",
                    )?,
                    encoding_version: i32_to_u32(
                        row.try_get("", "encoding_version").map_err(read)?,
                        "Git pool pack encoding",
                    )?,
                },
                request_id: row.try_get("", "request_id").map_err(read)?,
                head_oid: row.try_get("", "head_oid").map_err(read)?,
                prerequisites: row.try_get("", "prerequisites").map_err(read)?,
            })
        })
        .collect::<Result<Vec<_>, PostgresError>>()?;
    Ok(GitPoolPacks { packs, watermark })
}

impl crate::db::RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "git_pool_packs_after"))]
    pub async fn git_pool_packs_after(
        &self,
        repo_id: &str,
        watermark: Option<i64>,
    ) -> Result<GitPoolPacks, PostgresError> {
        git_pool_packs_after(self.db.as_ref(), repo_id, watermark).await
    }
}
