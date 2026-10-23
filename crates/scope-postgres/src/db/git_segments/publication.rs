use super::ledger::require_one_transition;
use crate::{
    db::integer_columns::{u32_to_i32, u64_to_i64},
    error::PostgresError,
};
use scope_domain::repository::git::GitSegmentRef;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

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
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_git_segment_uploads
             SET state = 'published',
                 updated_at_unix = GREATEST(updated_at_unix, $6)
             WHERE segment_id = $1 AND repo_id = $2
               AND state IN ('ready', 'published')
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
        .await?;
    require_one_transition(result.rows_affected(), &segment.segment_id, "published")
}
