use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0081_github_workflow_run_list"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE scope_github_workflow_runs ADD COLUMN request_id text;
             UPDATE scope_github_workflow_runs
                SET request_id = substring(head_branch FROM length('scope/requests/') + 1)
              WHERE head_branch ~ '^scope/requests/[^/]+$';
             CREATE INDEX idx_scope_github_workflow_runs_workflow_recent
                ON scope_github_workflow_runs (
                    repo_id, github_repository_id, workflow_name,
                    coalesce(run_started_at_unix, github_updated_at_unix) DESC,
                    github_run_id DESC
                );",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX idx_scope_github_workflow_runs_workflow_recent;
             ALTER TABLE scope_github_workflow_runs DROP COLUMN request_id;",
            )
            .await?;
        Ok(())
    }
}
