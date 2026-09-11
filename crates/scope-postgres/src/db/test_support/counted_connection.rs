use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, ExecResult, QueryResult, Statement,
};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct CountedConnection<'a> {
    pub db: &'a DatabaseConnection,
    pub queries: AtomicUsize,
}

#[sea_orm_migration::async_trait::async_trait]
impl ConnectionTrait for CountedConnection<'_> {
    fn get_database_backend(&self) -> DatabaseBackend {
        self.db.get_database_backend()
    }
    async fn execute(&self, statement: Statement) -> Result<ExecResult, DbErr> {
        self.db.execute(statement).await
    }
    async fn execute_unprepared(&self, sql: &str) -> Result<ExecResult, DbErr> {
        self.db.execute_unprepared(sql).await
    }
    async fn query_one(&self, statement: Statement) -> Result<Option<QueryResult>, DbErr> {
        self.queries.fetch_add(1, Ordering::Relaxed);
        self.db.query_one(statement).await
    }
    async fn query_all(&self, statement: Statement) -> Result<Vec<QueryResult>, DbErr> {
        self.queries.fetch_add(1, Ordering::Relaxed);
        self.db.query_all(statement).await
    }
}
