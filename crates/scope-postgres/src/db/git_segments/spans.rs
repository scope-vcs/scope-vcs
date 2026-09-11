use super::{super::entities, ledger::require_one_transition};
use crate::{
    db::integer_columns::{u32_to_i32, u64_to_i64},
    error::PostgresError,
};
use scope_domain::repository::git::{GitPackSpan, GitSegmentRef};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, QueryOrder, Statement,
};
use std::collections::BTreeMap;

pub(in crate::db) async fn load_git_pack_spans<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<GitPackSpan>, PostgresError>
where
    C: ConnectionTrait,
{
    let rows = entities::git_pack_span::Entity::find()
        .filter(entities::git_pack_span::Column::RepoId.eq(repo_id))
        .order_by_asc(entities::git_pack_span::Column::FirstSequence)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let segment_ids = rows
        .iter()
        .map(|row| row.segment_id.clone())
        .collect::<Vec<_>>();
    let uploads = if segment_ids.is_empty() {
        Vec::new()
    } else {
        entities::git_segment_upload::Entity::find()
            .filter(entities::git_segment_upload::Column::SegmentId.is_in(segment_ids))
            .all(conn)
            .await
            .map_err(PostgresError::internal)?
    };
    let mut segments = uploads
        .into_iter()
        .map(|upload| Ok((upload.segment_id.clone(), upload.ready_segment_ref()?)))
        .collect::<Result<BTreeMap<_, _>, PostgresError>>()?;
    rows.into_iter()
        .map(|row| {
            let segment = segments.remove(&row.segment_id).ok_or_else(|| {
                PostgresError::internal_message(format!(
                    "Git pack span references missing segment {}",
                    row.segment_id
                ))
            })?;
            row.try_into_domain(segment)
        })
        .collect()
}

pub(in crate::db) async fn publish_git_segment<C>(
    conn: &C,
    repo_id: &str,
    segment: &GitSegmentRef,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let result = conn
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_git_segment_uploads
             SET state = 'published',
                 updated_at_unix = GREATEST(updated_at_unix, $6)
             WHERE segment_id = $1 AND repo_id = $2
               AND state IN ('ready', 'published', 'retained')
               AND sha256 = $3 AND plaintext_bytes = $4 AND encoding_version = $5",
            [
                segment.segment_id.clone().into(),
                repo_id.into(),
                segment.sha256.clone().into(),
                u64_to_i64(segment.plaintext_bytes, "Git segment plaintext size")?.into(),
                u32_to_i32(segment.encoding_version, "Git segment encoding version")?.into(),
                u64_to_i64(now_unix, "Git segment publication time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    require_one_transition(result.rows_affected(), &segment.segment_id, "published")
}
