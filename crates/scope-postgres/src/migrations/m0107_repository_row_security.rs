use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

const REPOSITORY_OWNED_TABLES: [(&str, &str); 53] = [
    ("scope_cache_deletion_queue", "repository_id"),
    ("scope_cache_objects", "repository_id"),
    ("scope_cache_orphan_uploads", "repository_id"),
    ("scope_cache_references", "repository_id"),
    ("scope_cache_uploads", "repository_id"),
    ("scope_dependency_analyses", "repo_id"),
    ("scope_dependency_analysis_jobs", "repo_id"),
    ("scope_dependency_reports", "repo_id"),
    ("scope_file_changes", "repo_id"),
    ("scope_git_compaction_jobs", "repo_id"),
    ("scope_git_fetch_advertisements", "repo_id"),
    ("scope_git_heads", "repo_id"),
    ("scope_git_pool_packs", "repo_id"),
    ("scope_git_segment_uploads", "repo_id"),
    ("scope_github_check_refreshes", "repo_id"),
    ("scope_github_check_runs", "repo_id"),
    ("scope_github_connections", "repo_id"),
    ("scope_github_pull_requests", "repo_id"),
    ("scope_github_pushes", "repo_id"),
    ("scope_github_required_checks", "repo_id"),
    ("scope_github_review_bots", "repo_id"),
    ("scope_github_review_imports", "repo_id"),
    ("scope_github_run_import_counts", "repo_id"),
    ("scope_github_run_imports", "repo_id"),
    ("scope_github_setup_checks", "repo_id"),
    ("scope_github_workflow_job_reads", "repo_id"),
    ("scope_github_workflow_jobs", "repo_id"),
    ("scope_github_workflow_run_reads", "repo_id"),
    ("scope_github_workflow_runs", "repo_id"),
    ("scope_live_files", "repo_id"),
    ("scope_logical_commits", "repo_id"),
    ("scope_outbox_jobs", "repo_id"),
    ("scope_projection_files", "repo_id"),
    ("scope_projection_read_models", "repo_id"),
    ("scope_push_trigger_evaluations", "repo_id"),
    ("scope_repo_storage_cleanup_jobs", "repo_id"),
    ("scope_repositories", "id"),
    ("scope_repository_first_push_tokens", "repo_id"),
    ("scope_repository_git_push_tokens", "repo_id"),
    ("scope_repository_history_entries", "repo_id"),
    ("scope_repository_history_payloads", "repo_id"),
    ("scope_repository_invites", "repo_id"),
    ("scope_repository_landing_files", "repo_id"),
    ("scope_repository_members", "repo_id"),
    ("scope_repository_workflow_catalogs", "repo_id"),
    ("scope_repository_workflow_files", "repo_id"),
    ("scope_request_auto_merge_intents", "repo_id"),
    ("scope_request_media_attachments", "repository_id"),
    ("scope_request_media_cleanup_jobs", "repository_id"),
    ("scope_requests", "repo_id"),
    ("scope_runs", "repo_id"),
    ("scope_visibility_change_sets", "repo_id"),
    ("scope_visibility_changes", "repo_id"),
];

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0107_repository_row_security"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let statements = REPOSITORY_OWNED_TABLES
            .iter()
            .map(|(table, column)| {
                format!(
                    "ALTER TABLE {table} ENABLE ROW LEVEL SECURITY, FORCE ROW LEVEL SECURITY;
                    CREATE POLICY scope_repository_scope ON {table}
                        USING ({column} = (SELECT current_setting('scope.repository_id', true))
                            OR (SELECT current_setting('scope.system_reason', true)) <> '')
                        WITH CHECK ({column} = (SELECT current_setting('scope.repository_id', true))
                            OR (SELECT current_setting('scope.system_reason', true)) <> '');"
                )
            })
            .collect::<String>();
        manager
            .get_connection()
            .execute_unprepared(&statements)
            .await?;
        Ok(())
    }
}
