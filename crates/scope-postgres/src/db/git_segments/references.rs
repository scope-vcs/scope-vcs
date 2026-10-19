use super::ledger::require_text;
use crate::db::integer_columns::u64_to_i64;
use crate::error::PostgresError;
use scope_domain::repository::git::GitSegmentRef;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

const GIT_SEGMENT_REFERENCE_INSERT_BATCH: usize = 1_000;

pub(in crate::db) async fn insert_git_segment_references<'a, C>(
    conn: &C,
    ref_kind: &str,
    ref_id: &str,
    segments: impl IntoIterator<Item = &'a GitSegmentRef>,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    require_reference_kind(ref_kind)?;
    require_text(ref_id, "Git segment reference id")?;
    let segment_ids = segments
        .into_iter()
        .map(|segment| segment.segment_id.clone())
        .collect::<Vec<_>>();
    for chunk in segment_ids.chunks(GIT_SEGMENT_REFERENCE_INSERT_BATCH) {
        let rows = (0..chunk.len())
            .map(|index| format!("(${}, $1, $2)", index + 3))
            .collect::<Vec<_>>()
            .join(", ");
        let mut values = Vec::with_capacity(chunk.len() + 2);
        values.push(ref_kind.into());
        values.push(ref_id.into());
        values.extend(chunk.iter().cloned().map(sea_orm::Value::from));
        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "INSERT INTO scope_git_segment_references (segment_id, ref_kind, ref_id)
                 VALUES {rows} ON CONFLICT DO NOTHING"
            ),
            values,
        ))
        .await?;
    }
    Ok(())
}

pub(in crate::db) async fn release_git_segment_references<C>(
    conn: &C,
    ref_kind: &str,
    ref_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    require_reference_kind(ref_kind)?;
    require_text(ref_id, "Git segment reference id")?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "WITH released AS (
             DELETE FROM scope_git_segment_references
             WHERE ref_kind = $1 AND ref_id = $2
             RETURNING segment_id
         )
         UPDATE scope_git_segment_uploads uploads
         SET state = 'deleting',
             updated_at_unix = GREATEST(updated_at_unix, $3)
         WHERE uploads.segment_id IN (SELECT segment_id FROM released)
           AND uploads.state = 'retained'
           AND NOT EXISTS (
               SELECT 1 FROM scope_git_segments spans
               WHERE spans.segment_id = uploads.segment_id
           )
           AND NOT EXISTS (
               SELECT 1 FROM scope_git_segment_references refs
               WHERE refs.segment_id = uploads.segment_id
                 AND NOT (refs.ref_kind = $1 AND refs.ref_id = $2)
           )",
        [
            ref_kind.into(),
            ref_id.into(),
            u64_to_i64(now_unix, "Git segment reference release time")?.into(),
        ],
    ))
    .await?;
    Ok(())
}

fn require_reference_kind(ref_kind: &str) -> Result<(), PostgresError> {
    if matches!(ref_kind, "push_trigger_source" | "run_source") {
        Ok(())
    } else {
        Err(PostgresError::internal_message(format!(
            "unsupported Git segment reference kind {ref_kind}"
        )))
    }
}
