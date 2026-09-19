use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0056_request_auto_merge"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_request_auto_merge_intents (
                    id varchar PRIMARY KEY,
                    repo_id varchar NOT NULL
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    repository_incarnation_id text NOT NULL
                        REFERENCES scope_repositories(incarnation_id) ON DELETE CASCADE,
                    request_id varchar NOT NULL
                        REFERENCES scope_requests(id) ON DELETE CASCADE,
                    revision_id varchar NOT NULL,
                    head_oid varchar NOT NULL,
                    actor_user_id varchar NOT NULL
                        REFERENCES scope_users(id) ON DELETE RESTRICT,
                    status varchar NOT NULL,
                    reason varchar,
                    created_position bigint NOT NULL,
                    claim_token varchar,
                    claim_expires_at_unix bigint,
                    attempt integer NOT NULL DEFAULT 0,
                    next_attempt_at_unix bigint NOT NULL,
                    last_error text,
                    created_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT fk_scope_request_auto_merge_revision
                        FOREIGN KEY (request_id, revision_id)
                        REFERENCES scope_request_revisions(request_id, id) ON DELETE CASCADE,
                    CONSTRAINT scope_request_auto_merge_values CHECK (
                        length(btrim(id)) > 0 AND
                        length(btrim(repo_id)) > 0 AND
                        length(btrim(repository_incarnation_id)) > 0 AND
                        length(btrim(request_id)) > 0 AND
                        length(btrim(revision_id)) > 0 AND
                        length(head_oid) = 40 AND
                        length(btrim(actor_user_id)) > 0 AND
                        status IN ('Active', 'Cancelled', 'Stopped', 'Fulfilled') AND
                        reason IN (
                            'RequestChanged', 'RequestClosed', 'AccessRevoked',
                            'ChecksFailed', 'ChecksConfigurationError', 'MergeConflict',
                            'RequestBranchMissing'
                        ) AND
                        ((status = 'Stopped') = (reason IS NOT NULL)) AND
                        created_position > 0 AND
                        ((claim_token IS NULL) = (claim_expires_at_unix IS NULL)) AND
                        (status = 'Active' OR claim_token IS NULL) AND
                        attempt >= 0 AND next_attempt_at_unix >= 0 AND
                        (last_error IS NULL OR octet_length(last_error) BETWEEN 1 AND 8192) AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix
                    )
                );

                CREATE UNIQUE INDEX uq_scope_request_auto_merge_active
                    ON scope_request_auto_merge_intents(request_id)
                    WHERE status = 'Active';

                CREATE INDEX idx_scope_request_auto_merge_due
                    ON scope_request_auto_merge_intents(next_attempt_at_unix, created_at_unix, id)
                    WHERE status = 'Active';

                CREATE INDEX idx_scope_request_auto_merge_repo_actor
                    ON scope_request_auto_merge_intents(repo_id, actor_user_id)
                    WHERE status = 'Active';

                CREATE INDEX idx_scope_request_check_evaluations_checks
                    ON scope_request_check_evaluations USING gin (checks jsonb_path_ops);
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Request auto-merge is forward-only".into()))
    }
}
