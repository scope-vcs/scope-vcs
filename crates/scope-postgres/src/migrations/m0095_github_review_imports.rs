use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0095_github_review_imports"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_github_accounts (
                    github_user_id bigint NOT NULL,
                    login text NOT NULL,
                    account_type character varying NOT NULL,
                    avatar_url text,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_accounts_pkey PRIMARY KEY (github_user_id),
                    CONSTRAINT scope_github_account_values CHECK (
                        github_user_id > 0
                        AND length(btrim(login)) > 0
                        AND account_type IN ('User', 'Bot', 'Organization')
                        AND updated_at_unix >= 0
                    )
                );

                ALTER TABLE scope_request_discussions
                    ADD COLUMN author_github_user_id bigint,
                    ADD COLUMN source_url text,
                    ADD COLUMN resolved_by_github_user_id bigint,
                    ADD CONSTRAINT fk_scope_request_discussions_github_author
                        FOREIGN KEY (author_github_user_id)
                        REFERENCES scope_github_accounts(github_user_id) ON DELETE RESTRICT,
                    ADD CONSTRAINT fk_scope_request_discussions_github_resolver
                        FOREIGN KEY (resolved_by_github_user_id)
                        REFERENCES scope_github_accounts(github_user_id) ON DELETE RESTRICT,
                    ADD CONSTRAINT scope_request_discussion_author CHECK (
                        (author_github_user_id IS NULL AND source_url IS NULL)
                        OR (
                            author_github_user_id IS NOT NULL
                            AND author_user_id IS NULL
                            AND source_url IS NOT NULL
                            AND length(btrim(source_url)) > 0
                        )
                    ),
                    ADD CONSTRAINT scope_request_discussion_resolver CHECK (
                        resolved_by_user_id IS NULL OR resolved_by_github_user_id IS NULL
                    );

                ALTER TABLE scope_request_discussion_replies
                    ADD COLUMN author_github_user_id bigint,
                    ADD COLUMN source_url text,
                    ADD CONSTRAINT fk_scope_request_discussion_replies_github_author
                        FOREIGN KEY (author_github_user_id)
                        REFERENCES scope_github_accounts(github_user_id) ON DELETE RESTRICT,
                    ADD CONSTRAINT scope_request_discussion_reply_author CHECK (
                        (author_github_user_id IS NULL AND source_url IS NULL)
                        OR (
                            author_github_user_id IS NOT NULL
                            AND author_user_id IS NULL
                            AND source_url IS NOT NULL
                            AND length(btrim(source_url)) > 0
                        )
                    );

                CREATE UNIQUE INDEX idx_scope_request_discussions_github_client
                    ON scope_request_discussions (request_id, client_discussion_id)
                    WHERE author_github_user_id IS NOT NULL;

                CREATE UNIQUE INDEX idx_scope_request_discussion_replies_github_client
                    ON scope_request_discussion_replies (discussion_id, client_reply_id)
                    WHERE author_github_user_id IS NOT NULL;

                CREATE TABLE scope_github_review_imports (
                    request_id character varying NOT NULL,
                    subject character varying NOT NULL,
                    github_subject_id bigint NOT NULL,
                    repo_id character varying NOT NULL,
                    github_repository_id bigint NOT NULL,
                    pull_number bigint NOT NULL,
                    thread_status character varying,
                    github_actor_id bigint,
                    attempts integer NOT NULL,
                    next_attempt_at_unix bigint NOT NULL,
                    CONSTRAINT scope_github_review_imports_pkey
                        PRIMARY KEY (request_id, subject, github_subject_id),
                    CONSTRAINT fk_scope_github_review_imports_request
                        FOREIGN KEY (request_id) REFERENCES scope_requests(id) ON DELETE CASCADE,
                    CONSTRAINT fk_scope_github_review_imports_actor
                        FOREIGN KEY (github_actor_id)
                        REFERENCES scope_github_accounts(github_user_id) ON DELETE RESTRICT,
                    CONSTRAINT scope_github_review_import_values CHECK (
                        github_subject_id > 0
                        AND github_repository_id > 0
                        AND pull_number > 0
                        AND attempts >= 0
                        AND next_attempt_at_unix >= 0
                        AND (
                            (subject IN ('review', 'reply')
                                AND thread_status IS NULL
                                AND github_actor_id IS NULL)
                            OR (subject = 'thread'
                                AND thread_status IN ('Open', 'Resolved')
                                AND github_actor_id IS NOT NULL)
                        )
                    )
                );

                CREATE INDEX idx_scope_github_review_imports_due
                    ON scope_github_review_imports (next_attempt_at_unix);
                "#,
            )
            .await?;
        Ok(())
    }
}
