use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0046_drop_attempt_token_expiry"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The attempt token expired exactly when its lease did, so the column
        // and its equality CHECK stored one fact twice. The lease is the owner.
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_run_attempts
                    DROP CONSTRAINT scope_run_attempts_values,
                    DROP COLUMN token_expires_at_unix,
                    ADD CONSTRAINT scope_run_attempts_values CHECK (((number > 0) AND ((external_run_id IS NULL) OR (char_length(external_run_id) > 0)) AND ((runner_stop_claimed_at_unix IS NULL) OR (runner_stop_claimed_at_unix >= created_at_unix)) AND ((runner_stop_completed_at_unix IS NULL) OR ((runner_stop_claimed_at_unix IS NOT NULL) AND (runner_stop_completed_at_unix >= runner_stop_claimed_at_unix))) AND ((char_length(runtime_version) >= 1) AND (char_length(runtime_version) <= 128)) AND (char_length((token_hash)::text) = 64) AND ((token_hash)::text ~ '^[0-9A-Fa-f]+$'::text) AND ((state)::text = ANY ((ARRAY['dispatching'::character varying, 'running'::character varying, 'succeeded'::character varying, 'failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) AND (created_at_unix >= 0) AND (last_heartbeat_at_unix >= created_at_unix) AND (last_heartbeat_at_unix < lease_expires_at_unix) AND ((started_at_unix IS NULL) OR ((started_at_unix >= created_at_unix) AND (started_at_unix < lease_expires_at_unix))) AND ((completed_at_unix IS NULL) OR (completed_at_unix >= last_heartbeat_at_unix)) AND ((started_at_unix IS NULL) OR (completed_at_unix IS NULL) OR (completed_at_unix >= started_at_unix)) AND (log_bytes >= 0) AND (log_bytes <= 10485760) AND (((state)::text = ANY ((ARRAY['succeeded'::character varying, 'failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) = (completed_at_unix IS NOT NULL)) AND (((state)::text <> 'succeeded'::text) OR ((started_at_unix IS NOT NULL) AND (terminal_reason IS NULL))) AND (((state)::text <> ALL ((ARRAY['failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) OR (terminal_reason IS NOT NULL)) AND (((state)::text = ANY ((ARRAY['failed'::character varying, 'canceled'::character varying, 'lost'::character varying])::text[])) OR (terminal_reason IS NULL))));
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "attempt token expiry removal is forward-only".into(),
        ))
    }
}
