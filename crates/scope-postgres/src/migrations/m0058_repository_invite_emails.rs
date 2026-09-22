use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0058_repository_invite_emails"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_repository_invite_emails (
                    id varchar PRIMARY KEY,
                    -- Cleared, not cascaded, when the invite's repository is
                    -- deleted: the row still counts toward the owner's daily
                    -- allowance until it ages out.
                    invite_id varchar
                        REFERENCES scope_repository_invites(id) ON DELETE SET NULL,
                    requested_by_user_id varchar NOT NULL
                        REFERENCES scope_users(id) ON DELETE CASCADE,
                    state varchar NOT NULL,
                    attempts integer NOT NULL DEFAULT 0,
                    next_attempt_at_unix bigint NOT NULL,
                    claim_token varchar,
                    claim_expires_at_unix bigint,
                    provider_message_id varchar,
                    last_error text,
                    created_at_unix bigint NOT NULL,
                    updated_at_unix bigint NOT NULL,
                    CONSTRAINT scope_repository_invite_email_values CHECK (
                        length(btrim(id)) > 0 AND
                        state IN ('Queued', 'Sent', 'Failed') AND
                        attempts >= 0 AND next_attempt_at_unix >= 0 AND
                        ((claim_token IS NULL) = (claim_expires_at_unix IS NULL)) AND
                        (state = 'Queued' OR claim_token IS NULL) AND
                        (last_error IS NULL OR octet_length(last_error) BETWEEN 1 AND 8192) AND
                        created_at_unix >= 0 AND updated_at_unix >= created_at_unix
                    )
                );

                CREATE INDEX idx_scope_repository_invite_emails_invite
                    ON scope_repository_invite_emails(invite_id, created_at_unix);

                CREATE INDEX idx_scope_repository_invite_emails_requester
                    ON scope_repository_invite_emails(requested_by_user_id, created_at_unix);

                CREATE INDEX idx_scope_repository_invite_emails_due
                    ON scope_repository_invite_emails(next_attempt_at_unix, created_at_unix, id)
                    WHERE state = 'Queued';
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Repository invite emails are forward-only".into(),
        ))
    }
}
