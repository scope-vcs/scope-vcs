use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0061_public_request_check_source"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Public trees omit workflow definitions. Their old empty evaluations
        // cannot establish that main requires no checks. The next read or merge
        // evaluates the saved revision against the accepted main catalog.
        manager
            .get_connection()
            .execute_unprepared(
                "DELETE FROM scope_request_check_evaluations AS evaluation
                 USING scope_requests AS request
                 WHERE evaluation.request_id = request.id
                   AND request.audience = 'Public'
                   AND request.closed_at_unix IS NULL
                   AND request.merged_at_unix IS NULL
                   AND evaluation.state = 'no-checks'",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "Public request check source repair is forward-only".into(),
        ))
    }
}
