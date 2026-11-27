use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0123_reconcile_visibility_labels"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let now = conn
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT extract(epoch FROM now())::bigint AS now",
            ))
            .await?
            .ok_or_else(|| DbErr::Custom("migration time unavailable".into()))?
            .try_get::<i64>("", "now")?;
        crate::db::repository_rows::reconcile_drifted_visibility_labels(conn, now)
            .await
            .map_err(|error| DbErr::Custom(error.to_string()))
    }
}
