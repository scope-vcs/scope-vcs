use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0098_live_file_views"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_live_files ADD COLUMN view text;

                UPDATE scope_live_files AS live
                SET view = rule.view
                FROM (
                    SELECT DISTINCT ON (repository.id, entry.value->>'path')
                        repository.id AS repo_id,
                        entry.value->>'path' AS path,
                        entry.value->>'view' AS view
                    FROM scope_repositories AS repository,
                         jsonb_array_elements(repository.policy->'rules')
                             WITH ORDINALITY AS entry
                    ORDER BY repository.id, entry.value->>'path', entry.ordinality DESC
                ) AS rule
                WHERE live.repo_id = rule.repo_id
                  AND live.path = rule.path;

                UPDATE scope_repositories AS repository
                SET policy = jsonb_set(
                    repository.policy,
                    '{rules}',
                    COALESCE((
                        SELECT jsonb_agg(rule.value ORDER BY rule.ordinality)
                        FROM jsonb_array_elements(repository.policy->'rules')
                             WITH ORDINALITY AS rule
                        WHERE NOT EXISTS (
                            SELECT 1
                            FROM scope_live_files AS live
                            WHERE live.repo_id = repository.id
                              AND live.path = rule.value->>'path'
                        )
                    ), '[]'::jsonb)
                )
                WHERE jsonb_typeof(repository.policy->'rules') = 'array';
                "#,
            )
            .await?;
        Ok(())
    }
}
