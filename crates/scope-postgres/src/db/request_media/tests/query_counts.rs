use super::*;
use sea_orm::{DatabaseConnection, DbErr, ExecResult, QueryResult};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountedConnection<'a> {
    db: &'a DatabaseConnection,
    queries: AtomicUsize,
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

#[tokio::test]
async fn batch_metadata_query_count_is_independent_of_attachment_count() {
    let fixture = fixture();
    start_request(&fixture, "batch_request", "batch-request", 1).await;
    for index in 0..8 {
        upload_attachment(
            &fixture,
            "batch_request",
            &format!("batch_{index}"),
            10 + index * 10,
        )
        .await;
        if !matches!(index, 0 | 7) {
            continue;
        }
        let conn = CountedConnection {
            db: fixture.store.db.as_ref(),
            queries: AtomicUsize::new(0),
        };
        let attachments =
            super::super::persistence::attachments_for_request(&conn, "batch_request")
                .await
                .unwrap();
        let bindings = super::super::persistence::bindings_for_request(&conn, "batch_request")
            .await
            .unwrap();
        assert_eq!(attachments.len(), index as usize + 1);
        assert!(attachments.iter().all(|attachment| {
            attachment
                .original
                .as_ref()
                .is_some_and(|original| original.size_bytes == 4)
        }));
        assert!(bindings.is_empty());
        assert_eq!(conn.queries.load(Ordering::Relaxed), 3);
    }
}
