use crate::error::PostgresError;
use std::{
    cell::RefCell,
    sync::{
        Arc, Once,
        atomic::{AtomicU64, Ordering},
    },
};
use tracing::{
    Metadata, Subscriber,
    span::{Attributes, Id, Record},
};

tokio::task_local! {
    static STATEMENTS: RefCell<Vec<&'static str>>;
}

#[derive(Default)]
struct ReadSubscriber(AtomicU64);

impl Subscriber for ReadSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        matches!(
            metadata.name(),
            "sea_orm.begin"
                | "sea_orm.commit"
                | "sea_orm.rollback"
                | "sea_orm.query_one"
                | "sea_orm.query_all"
                | "sea_orm.execute"
                | "sea_orm.execute_unprepared"
        )
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        STATEMENTS
            .try_with(|statements| {
                statements.borrow_mut().push(attributes.metadata().name());
            })
            .ok();
        Id::from_u64(self.0.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

pub(crate) async fn assert_round_trips<T>(
    expected: usize,
    read: impl Future<Output = Result<T, PostgresError>>,
) -> Result<T, PostgresError> {
    static SUBSCRIBER: Once = Once::new();
    SUBSCRIBER.call_once(|| {
        tracing::subscriber::set_global_default(ReadSubscriber::default()).unwrap();
    });
    STATEMENTS
        .scope(RefCell::default(), async {
            let result = read.await;
            STATEMENTS.with(|statements| {
                let statements = statements.borrow();
                let round_trips: usize = statements
                    .iter()
                    .map(|name| match *name {
                        "sea_orm.begin" => 2,
                        _ => 1,
                    })
                    .sum();
                assert_eq!(
                    round_trips,
                    expected,
                    "{statements:?}, error: {:?}",
                    result.as_ref().err()
                );
            });
            result
        })
        .await
}

#[tokio::test]
async fn round_trip_measurement_isolates_concurrent_statements_and_callsite_registration() {
    use super::TestDatabaseTarget;
    use sea_orm::{
        AccessMode, ConnectionTrait, Database, DatabaseBackend, IsolationLevel, Statement,
        TransactionTrait,
    };
    use tokio::sync::Barrier;

    let target = TestDatabaseTarget::required().unwrap();
    let db = Arc::new(Database::connect(target.database_url).await.unwrap());
    let barrier = Arc::new(Barrier::new(2));
    let other_db = Arc::clone(&db);
    let other_barrier = Arc::clone(&barrier);
    let other_test = super::test_runtime().spawn(async move {
        other_barrier.wait().await;
        let transaction = other_db.begin().await.unwrap();
        transaction
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT 1",
            ))
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        tracing::callsite::rebuild_interest_cache();
        other_barrier.wait().await;
    });

    let result = assert_round_trips(5, async {
        let transaction = db
            .begin_with_config(
                Some(IsolationLevel::RepeatableRead),
                Some(AccessMode::ReadOnly),
            )
            .await?;
        barrier.wait().await;
        barrier.wait().await;
        let row = transaction
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT 42 AS answer",
            ))
            .await?
            .unwrap();
        let rows = transaction
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT 43 AS answer",
            ))
            .await?;
        transaction.commit().await?;
        Ok((
            row.try_get::<i32>("", "answer")?,
            rows[0].try_get::<i32>("", "answer")?,
        ))
    })
    .await
    .unwrap();
    other_test.await.unwrap();
    assert_eq!(result, (42, 43));
}
