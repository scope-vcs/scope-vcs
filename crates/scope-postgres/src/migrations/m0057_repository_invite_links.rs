use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0057_repository_invite_links"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_repository_invite_links (
                    token_hash varchar PRIMARY KEY,
                    invite_id varchar NOT NULL
                        REFERENCES scope_repository_invites(id) ON DELETE CASCADE,
                    CONSTRAINT scope_repository_invite_link_values CHECK (
                        length(btrim(token_hash)) > 0
                    )
                );

                CREATE INDEX idx_scope_repository_invite_links_invite
                    ON scope_repository_invite_links(invite_id);

                -- Every existing invite keeps its one link, so pending links
                -- sent before this migration still open.
                INSERT INTO scope_repository_invite_links (token_hash, invite_id)
                SELECT token_hash, id FROM scope_repository_invites;

                -- An invite's state now follows from its timestamps. Make the
                -- timestamps agree with the stored state before dropping it.
                UPDATE scope_repository_invites
                SET revoked_at_unix = updated_at_unix
                WHERE state = 'Revoked' AND revoked_at_unix IS NULL;

                UPDATE scope_repository_invites
                SET accepted_at_unix = updated_at_unix
                WHERE state = 'Accepted' AND accepted_at_unix IS NULL;

                UPDATE scope_repository_invites
                SET expires_at_unix = updated_at_unix
                WHERE state = 'Expired' AND expires_at_unix > updated_at_unix;

                DROP INDEX idx_scope_repository_invites_token_hash;

                ALTER TABLE scope_repository_invites
                    DROP COLUMN token_hash,
                    DROP COLUMN state;
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Repository invite links are forward-only".into(),
        ))
    }
}
