use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0111_remove_native_runs"
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
        let mut after = String::new();
        loop {
            let rows = conn
                .query_all_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "SELECT id, source->'object' AS object FROM scope_runs
                     WHERE id > $1 AND source ? 'object'
                     ORDER BY id LIMIT 500",
                    [after.clone().into()],
                ))
                .await?;
            let Some(last) = rows.last() else {
                break;
            };
            after = last.try_get::<String>("", "id")?;
            let blobs = rows
                .into_iter()
                .map(|row| {
                    let value = row.try_get::<serde_json::Value>("", "object")?;
                    serde_json::from_value(value).map_err(|error| DbErr::Custom(error.to_string()))
                })
                .collect::<Result<Vec<scope_domain::content::SourceBlob>, DbErr>>()?;
            crate::db::cleanup_queue::queue::queue_pending_source_blob_deletion_rows(
                conn, blobs, now,
            )
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))?;
        }
        conn.execute_unprepared(
            r#"
            DELETE FROM scope_object_references
                WHERE ref_kind IN ('run_source', 'push_trigger_source');
            ALTER TABLE scope_outbox_jobs DROP CONSTRAINT scope_outbox_jobs_push_workflow_schema_v5;
            DELETE FROM scope_outbox_jobs WHERE kind = 'push_main_trigger_evaluation';
            ALTER TABLE scope_request_check_evaluations
                DROP CONSTRAINT scope_request_check_evaluation_values;
            DELETE FROM scope_request_check_evaluations
                WHERE checks @? '$[*] ? (@.provider != "github")'
                   OR (state = 'configuration-error' AND github_repository_id IS NULL);
            UPDATE scope_request_check_evaluations SET checks = (
                SELECT COALESCE(jsonb_agg(jsonb_build_object('name', check_entry->'name') ORDER BY position), '[]'::jsonb)
                FROM jsonb_array_elements(checks) WITH ORDINALITY AS entries(check_entry, position)
            );
            ALTER TABLE scope_request_check_evaluations
                ADD CONSTRAINT scope_request_check_evaluation_values CHECK (
                    length(head_oid) = 40 AND length(tested_oid) = 40 AND
                    state IN ('no-checks', 'awaiting-approval', 'started', 'configuration-error') AND
                    jsonb_typeof(checks) = 'array' AND
                    NOT checks @? '$[*] ? (!exists(@.name) || @.name.type() != "string")' AND
                    created_at_unix >= 0 AND updated_at_unix >= created_at_unix AND
                    ((state = 'configuration-error') = (length(btrim(coalesce(message, ''))) > 0)) AND
                    (state <> 'no-checks' OR jsonb_array_length(checks) = 0) AND
                    (state NOT IN ('awaiting-approval', 'started') OR jsonb_array_length(checks) > 0)
                );
            DROP TABLE
                scope_run_logs,
                scope_run_attempt_cache_setups,
                scope_run_attempt_caches,
                scope_run_attempt_steps,
                scope_run_attempts,
                scope_run_jobs,
                scope_runs,
                scope_workflow_revisions,
                scope_push_trigger_evaluations,
                scope_repository_workflow_files,
                scope_repository_workflow_catalogs,
                scope_native_runs_accounts,
                scope_cache_deletion_queue,
                scope_cache_references,
                scope_cache_uploads,
                scope_cache_orphan_uploads,
                scope_cache_objects;
            DROP FUNCTION scope_check_repository_workflow_catalog_rejection();
            DROP FUNCTION scope_check_repository_workflow_file();
            "#,
        )
        .await?;
        Ok(())
    }
}
