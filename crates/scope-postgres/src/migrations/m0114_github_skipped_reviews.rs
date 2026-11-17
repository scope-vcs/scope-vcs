use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0114_github_skipped_reviews"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TABLE scope_github_skipped_reviews (
                    request_id character varying NOT NULL,
                    review_id bigint NOT NULL,
                    repo_id character varying NOT NULL,
                    github_repository_id bigint NOT NULL,
                    pull_number bigint NOT NULL,
                    github_user_id bigint NOT NULL,
                    skipped_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_skipped_reviews_pkey PRIMARY KEY (request_id, review_id),
                    CONSTRAINT fk_scope_github_skipped_reviews_request
                        FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE,
                    CONSTRAINT fk_scope_github_skipped_reviews_author
                        FOREIGN KEY (github_user_id)
                        REFERENCES scope_github_accounts(github_user_id) ON DELETE RESTRICT,
                    CONSTRAINT scope_github_skipped_review_values CHECK (
                        review_id > 0
                        AND github_repository_id > 0
                        AND pull_number > 0
                        AND skipped_at_unix >= 0
                    )
                );
                CREATE INDEX idx_scope_github_skipped_reviews_author
                    ON scope_github_skipped_reviews (repo_id, github_user_id)",
            )
            .await?;
        Ok(())
    }
}
