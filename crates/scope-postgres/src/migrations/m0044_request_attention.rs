use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0044_request_attention"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_request_claims (
                    request_id varchar PRIMARY KEY
                        REFERENCES scope_requests(id) ON DELETE CASCADE,
                    claimer_user_id varchar NOT NULL
                        REFERENCES scope_users(id) ON DELETE RESTRICT,
                    claimed_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_request_claim_values CHECK (
                        claimed_at_unix >= 0 AND updated_at_unix >= claimed_at_unix
                    )
                );

                CREATE TABLE scope_request_attention_states (
                    request_id varchar NOT NULL
                        REFERENCES scope_requests(id) ON DELETE CASCADE,
                    user_id varchar NOT NULL
                        REFERENCES scope_users(id) ON DELETE CASCADE,
                    state varchar NOT NULL,
                    reason varchar NOT NULL,
                    through_activity_version bigint NOT NULL,
                    snoozed_until_unix bigint,
                    updated_at_unix bigint NOT NULL,
                    PRIMARY KEY (request_id, user_id),
                    CONSTRAINT scope_request_attention_values CHECK (
                        state IN ('active', 'waiting', 'snoozed', 'settled') AND
                        reason IN ('claimed', 'new_activity', 'restored', 'waiting', 'snoozed', 'settled') AND
                        through_activity_version >= 0 AND updated_at_unix >= 0 AND
                        ((state = 'snoozed') = (snoozed_until_unix IS NOT NULL)) AND
                        (snoozed_until_unix IS NULL OR snoozed_until_unix > updated_at_unix)
                    )
                );

                CREATE INDEX idx_scope_request_claims_claimer
                    ON scope_request_claims (claimer_user_id, request_id);
                CREATE INDEX idx_scope_request_attention_queue
                    ON scope_request_attention_states
                    (user_id, state, snoozed_until_unix, request_id);
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Request attention is forward-only".into()))
    }
}
