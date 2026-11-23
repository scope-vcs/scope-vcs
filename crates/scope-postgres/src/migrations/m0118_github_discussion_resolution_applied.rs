use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0118_github_discussion_resolution_applied"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE scope_request_discussions
                    ADD COLUMN github_resolution_applied boolean NOT NULL DEFAULT false;
                UPDATE scope_request_discussions discussion
                   SET github_resolution_applied = true
                 WHERE discussion.author_github_user_id IS NOT NULL
                   AND (discussion.status = 'Resolved' OR EXISTS (
                        SELECT 1 FROM scope_request_events event
                         WHERE event.request_id = discussion.request_id
                           AND event.kind = 'DiscussionReopened'
                           AND event.payload -> 'DiscussionReopened' ->> 'discussion_id'
                               = discussion.id
                   ))",
            )
            .await?;
        Ok(())
    }
}
