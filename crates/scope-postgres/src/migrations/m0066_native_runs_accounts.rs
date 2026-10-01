use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0066_native_runs_accounts"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- Scope's hosted runner serves only the accounts an operator
                -- lists. A repository may use native runs while its owning
                -- account is listed.
                CREATE TABLE scope_native_runs_accounts (
                    user_id varchar PRIMARY KEY
                        REFERENCES scope_users(id) ON DELETE CASCADE,
                    added_at_unix bigint NOT NULL,
                    note text,
                    CONSTRAINT scope_native_runs_account_values CHECK (
                        added_at_unix >= 0 AND
                        (note IS NULL OR
                            (length(btrim(note)) > 0 AND char_length(note) <= 500))
                    )
                );

                -- Owners whose repositories already use native runs keep them.
                INSERT INTO scope_native_runs_accounts (user_id, added_at_unix, note)
                SELECT DISTINCT repo.owner_user_id,
                       extract(epoch FROM now())::bigint,
                       'Used native runs before the allowlist'
                FROM scope_runs run
                JOIN scope_repositories repo ON repo.id = run.repo_id;
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Native runs accounts are forward-only".into(),
        ))
    }
}
