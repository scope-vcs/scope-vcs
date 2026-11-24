use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0121_request_links"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE scope_request_links (
                    resolving_request_id character varying NOT NULL,
                    resolved_request_id character varying NOT NULL,
                    created_at_unix bigint NOT NULL,
                    CONSTRAINT scope_request_links_pkey
                        PRIMARY KEY (resolving_request_id, resolved_request_id),
                    CONSTRAINT fk_scope_request_links_resolving
                        FOREIGN KEY (resolving_request_id)
                        REFERENCES scope_requests(id) ON DELETE CASCADE,
                    CONSTRAINT fk_scope_request_links_resolved
                        FOREIGN KEY (resolved_request_id)
                        REFERENCES scope_requests(id) ON DELETE CASCADE,
                    CONSTRAINT scope_request_link_values CHECK (
                        resolving_request_id <> resolved_request_id
                        AND created_at_unix >= 0
                    )
                );
                CREATE INDEX idx_scope_request_links_resolved
                    ON scope_request_links (resolved_request_id);

                ALTER TABLE scope_requests
                    ADD COLUMN close_named_request_id character varying,
                    ADD CONSTRAINT fk_scope_requests_close_named_request
                        FOREIGN KEY (close_named_request_id)
                        REFERENCES scope_requests(id) ON DELETE SET NULL,
                    ADD CONSTRAINT scope_request_close_named_request CHECK (
                        close_named_request_id IS NULL
                        OR (
                            close_reason IN ('completed', 'duplicate')
                            AND close_named_request_id <> id
                        )
                    );
                CREATE INDEX idx_scope_requests_close_named_request
                    ON scope_requests (close_named_request_id)
                    WHERE close_named_request_id IS NOT NULL;
                "#,
            )
            .await?;
        Ok(())
    }
}
