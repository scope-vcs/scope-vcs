use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0077_custom_views"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
ALTER TABLE scope_visibility_change_sets
    ADD COLUMN views_before jsonb,
    ADD COLUMN views_after jsonb,
    ADD CONSTRAINT scope_visibility_change_set_views CHECK (
        (views_before IS NULL) = (views_after IS NULL) AND
        (views_before IS NULL OR
            (jsonb_typeof(views_before) = 'array' AND jsonb_typeof(views_after) = 'array')));

ALTER TABLE scope_projection_read_models ADD COLUMN views jsonb NOT NULL DEFAULT
    '[{"id":"public","name":"Public","includes":[],"readers":"anyone"},
      {"id":"private","name":"Private","includes":"all","readers":"assigned"}]'::jsonb;
ALTER TABLE scope_projection_read_models ALTER COLUMN views DROP DEFAULT;
ALTER TABLE scope_projection_read_models ADD CONSTRAINT scope_projection_read_model_views
    CHECK (jsonb_typeof(views) = 'array' AND jsonb_array_length(views) > 0);

UPDATE scope_repositories SET repo_config = jsonb_set(repo_config, '{views}', (
    SELECT jsonb_agg(
        CASE WHEN view->>'readers' = 'members'
            THEN jsonb_set(view, '{readers}', '"assigned"')
            ELSE view END
        ORDER BY ordinal)
    FROM jsonb_array_elements(repo_config->'views') WITH ORDINALITY AS views(view, ordinal)))
WHERE repo_config->'views' @> '[{"readers":"members"}]';
"#,
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("Custom views are forward-only".into()))
    }
}
