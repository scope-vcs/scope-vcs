use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0068_github_connections"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_github_connections (
                    repo_id varchar PRIMARY KEY
                        REFERENCES scope_repositories(id) ON DELETE CASCADE,
                    installation_id bigint NOT NULL,
                    github_repository_id bigint NOT NULL,
                    github_full_name varchar NOT NULL,
                    -- Cleared when the account is deleted; the link stays.
                    connected_by_user_id varchar
                        REFERENCES scope_users(id) ON DELETE SET NULL,
                    connected_at_unix bigint NOT NULL,
                    status varchar NOT NULL,
                    disconnect_reason varchar,
                    disconnected_at_unix bigint,
                    CONSTRAINT scope_github_connection_values CHECK (
                        installation_id > 0 AND github_repository_id > 0 AND
                        length(btrim(github_full_name)) > 0 AND connected_at_unix >= 0 AND
                        status IN ('Connected', 'Disconnected') AND
                        ((status = 'Connected') = (disconnect_reason IS NULL)) AND
                        ((disconnect_reason IS NULL) = (disconnected_at_unix IS NULL)) AND
                        (disconnect_reason IS NULL OR disconnect_reason IN
                            ('AppUninstalled', 'InstallationSuspended', 'RepositoryRemoved')) AND
                        (disconnected_at_unix IS NULL OR disconnected_at_unix >= connected_at_unix)
                    )
                );

                -- A GitHub repository checks for one Scope repository at a time.
                CREATE UNIQUE INDEX idx_scope_github_connections_connected_repository
                    ON scope_github_connections(github_repository_id)
                    WHERE status = 'Connected';

                CREATE INDEX idx_scope_github_connections_installation
                    ON scope_github_connections(installation_id)
                    WHERE status = 'Connected';
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("GitHub connections are forward-only".into()))
    }
}
