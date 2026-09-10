use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0045_dependency_analysis"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_dependency_analyses (
                    repo_id text PRIMARY KEY REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    incarnation_id text NOT NULL,
                    head_oid text NOT NULL,
                    analyzer_version text NOT NULL,
                    analysis jsonb NOT NULL,
                    completed_at_unix bigint NOT NULL,
                    CONSTRAINT scope_dependency_analyses_values CHECK (
                        length(btrim(incarnation_id)) > 0 AND
                        length(btrim(head_oid)) > 0 AND
                        length(btrim(analyzer_version)) BETWEEN 1 AND 200 AND
                        jsonb_typeof(analysis) = 'object' AND
                        completed_at_unix >= 0
                    )
                );

                CREATE TABLE scope_dependency_reports (
                    repo_id text PRIMARY KEY REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    incarnation_id text NOT NULL,
                    repo_version bigint NOT NULL,
                    head_oid text NOT NULL,
                    analyzer_version text NOT NULL,
                    report jsonb NOT NULL,
                    completed_at_unix bigint NOT NULL,
                    CONSTRAINT scope_dependency_reports_values CHECK (
                        length(btrim(incarnation_id)) > 0 AND
                        repo_version > 0 AND
                        length(btrim(head_oid)) > 0 AND
                        length(btrim(analyzer_version)) BETWEEN 1 AND 200 AND
                        jsonb_typeof(report) = 'object' AND
                        completed_at_unix >= 0
                    )
                );

                CREATE TABLE scope_dependency_analysis_jobs (
                    repo_id text PRIMARY KEY REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    incarnation_id text NOT NULL,
                    repo_version bigint NOT NULL,
                    head_oid text NOT NULL,
                    analyzer_version text NOT NULL,
                    attempts integer NOT NULL,
                    next_run_at_unix bigint NOT NULL,
                    lease_generation text,
                    lease_owner text,
                    lease_expires_at_unix bigint,
                    last_error text,
                    created_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_dependency_analysis_jobs_values CHECK (
                        length(btrim(incarnation_id)) > 0 AND
                        repo_version > 0 AND
                        length(btrim(head_oid)) > 0 AND
                        length(btrim(analyzer_version)) BETWEEN 1 AND 200 AND
                        attempts >= 0 AND next_run_at_unix >= 0 AND
                        ((lease_generation IS NULL) = (lease_owner IS NULL)) AND
                        ((lease_generation IS NULL) = (lease_expires_at_unix IS NULL)) AND
                        (lease_generation IS NULL OR length(btrim(lease_generation)) > 0) AND
                        (lease_owner IS NULL OR length(btrim(lease_owner)) > 0) AND
                        (lease_expires_at_unix IS NULL OR lease_expires_at_unix >= 0) AND
                        (last_error IS NULL OR length(last_error) BETWEEN 1 AND 2000) AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix
                    )
                );

                CREATE INDEX scope_dependency_analysis_jobs_due
                    ON scope_dependency_analysis_jobs (next_run_at_unix, updated_at_unix, repo_id);
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Dependency analysis persistence is forward-only".into(),
        ))
    }
}
