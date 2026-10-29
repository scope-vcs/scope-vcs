use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0104_request_revision_diff_stats"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_request_revision_diff_stats (
                    revision_id character varying PRIMARY KEY
                        REFERENCES scope_request_revisions(id) ON DELETE CASCADE,
                    additions bigint NOT NULL,
                    deletions bigint NOT NULL,
                    changed_files bigint NOT NULL,
                    files_listed boolean NOT NULL,
                    CONSTRAINT scope_request_revision_diff_stat_values CHECK (
                        additions >= 0 AND deletions >= 0 AND changed_files >= 0
                    )
                );

                CREATE TABLE scope_request_revision_diff_files (
                    revision_id character varying NOT NULL
                        REFERENCES scope_request_revision_diff_stats(revision_id) ON DELETE CASCADE,
                    path text NOT NULL,
                    additions bigint,
                    deletions bigint,
                    PRIMARY KEY (revision_id, path),
                    CONSTRAINT scope_request_revision_diff_file_values CHECK (
                        length(path) > 1
                        AND (additions IS NULL) = (deletions IS NULL)
                        AND (additions IS NULL OR (additions >= 0 AND deletions >= 0))
                    )
                );
                "#,
            )
            .await?;
        Ok(())
    }
}
