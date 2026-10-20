use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0092_request_discussion_line_anchors"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE scope_request_discussions
                    ADD COLUMN line_side character varying,
                    ADD COLUMN start_line integer,
                    ADD COLUMN end_line integer,
                    DROP CONSTRAINT scope_request_discussion_values,
                    ADD CONSTRAINT scope_request_discussion_values CHECK (
                        opened_position > 0
                        AND last_activity_position >= opened_position
                        AND status IN ('Open', 'Resolved')
                        AND length(btrim(body_markdown)) > 0
                        AND created_at_unix >= 0
                        AND (resolved_at_unix IS NULL OR resolved_at_unix >= 0)
                        AND (commit_oid IS NULL OR revision_id IS NOT NULL)
                        AND (path IS NULL OR revision_id IS NOT NULL)
                    ),
                    ADD CONSTRAINT scope_request_discussion_line_anchor CHECK (
                        (line_side IS NULL AND start_line IS NULL AND end_line IS NULL)
                        OR (
                            line_side IN ('Old', 'New')
                            AND path IS NOT NULL
                            AND start_line >= 1
                            AND end_line >= start_line
                        )
                    );
                "#,
            )
            .await?;
        Ok(())
    }
}
