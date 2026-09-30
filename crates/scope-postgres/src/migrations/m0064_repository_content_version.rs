use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0064_repository_content_version"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                -- Projections and history views are keyed on the content
                -- version, so changes to members, invites, and metadata no
                -- longer invalidate them. Starting from the change version
                -- keeps every existing read model current.
                ALTER TABLE scope_repositories ADD COLUMN content_version bigint;
                UPDATE scope_repositories SET content_version = change_version;
                ALTER TABLE scope_repositories
                    ALTER COLUMN content_version SET NOT NULL,
                    ADD CONSTRAINT scope_repositories_content_version_bounds
                        CHECK (content_version BETWEEN 0 AND change_version);
                "#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Repository content versions are forward-only".into(),
        ))
    }
}
