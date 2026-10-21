use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0093_github_main_publication"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_github_connections
                    ADD COLUMN publishes_main boolean NOT NULL DEFAULT false,
                    ADD CONSTRAINT scope_github_connection_main_publication
                        CHECK (NOT publishes_main OR status = 'Connected');
                ALTER TABLE scope_github_pushes
                    DROP CONSTRAINT scope_github_push_values,
                    ADD CONSTRAINT scope_github_push_values CHECK (
                        ((request_id IS NOT NULL AND ref = 'refs/heads/scope/requests/' || request_id)
                            OR (request_id IS NULL AND ref IN ('refs/heads/scope/setup-check', 'refs/heads/main')))
                        AND (ref <> 'refs/heads/main' OR kind = 'push')
                        AND sequence > 0 AND installation_id > 0 AND github_repository_id > 0
                        AND length(btrim(github_full_name)) > 0
                        AND kind IN ('push', 'delete')
                        AND (kind = 'push') = (target_oid IS NOT NULL)
                        AND (target_oid IS NULL OR length(target_oid) = 40)
                        AND state IN ('queued', 'running', 'succeeded', 'failed')
                        AND (state = 'running') = (lease_until_unix IS NOT NULL)
                        AND (state = 'running') = (claim_token IS NOT NULL)
                        AND (state <> 'failed' OR last_error IS NOT NULL)
                        AND attempts >= 0 AND next_attempt_at_unix >= 0 AND created_at_unix >= 0
                        AND updated_at_unix >= created_at_unix
                    );
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Migration(
            "GitHub main publication is forward-only".into(),
        ))
    }
}
