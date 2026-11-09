use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0107_delete_request_bundles"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let now = conn
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT extract(epoch FROM now())::bigint AS now",
            ))
            .await?
            .ok_or_else(|| DbErr::Custom("migration time unavailable".into()))?
            .try_get::<i64>("", "now")? as u64;
        let mut offset = 0_i64;
        loop {
            let rows = conn
                .query_all_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT 0 AS kind, id, git_snapshot FROM scope_requests WHERE git_snapshot IS NOT NULL
                 UNION ALL SELECT 1, id, git_snapshot FROM scope_request_revisions
                 ORDER BY kind, id LIMIT 500 OFFSET $1",
                    [offset.into()],
                ))
                .await?;
            if rows.is_empty() {
                break;
            }
            offset += rows.len() as i64;
            let blobs = rows
                .into_iter()
                .map(|row| {
                    let value = row.try_get::<serde_json::Value>("", "git_snapshot")?;
                    serde_json::from_value(value).map_err(|error| DbErr::Custom(error.to_string()))
                })
                .collect::<Result<Vec<scope_domain::content::SourceBlob>, DbErr>>()?;
            let keys = blobs
                .iter()
                .map(|blob| {
                    serde_json::to_string(&blob.content_ref)
                        .map_err(|error| DbErr::Custom(error.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let retained = conn.query_all_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
                "SELECT object_key FROM scope_object_references WHERE ref_kind = 'run_source' AND object_key = ANY($1)",
                [keys.clone().into()])).await?.into_iter().map(|row| row.try_get::<String>("", "object_key"))
                .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
            let blobs = blobs
                .into_iter()
                .zip(keys)
                .filter_map(|(blob, key)| (!retained.contains(&key)).then_some(blob));
            crate::db::cleanup_queue::queue::queue_pending_source_blob_deletion_rows(
                conn, blobs, now,
            )
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))?;
        }
        conn.execute_unprepared("UPDATE scope_git_fetch_advertisements SET requests = (
                SELECT COALESCE(jsonb_agg(jsonb_build_object('id', entry->'id', 'view', entry->'view')), '[]'::jsonb)
                FROM jsonb_array_elements(CASE WHEN jsonb_typeof(requests) = 'array' THEN requests ELSE '[]'::jsonb END) entry
                WHERE jsonb_typeof(entry->'id') = 'string' AND jsonb_typeof(entry->'view') = 'string'
            ) WHERE requests IS NOT NULL;
            ALTER TABLE scope_requests ADD COLUMN pushed_at_unix bigint;
            UPDATE scope_requests SET pushed_at_unix = updated_at_unix WHERE git_snapshot IS NOT NULL;
            DELETE FROM scope_object_references WHERE ref_kind IN ('request_snapshot', 'request_revision_snapshot');
            ALTER TABLE scope_requests DROP COLUMN git_snapshot;
            ALTER TABLE scope_request_revisions DROP COLUMN git_snapshot;").await?;
        Ok(())
    }
}
