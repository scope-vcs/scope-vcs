use crate::error::PostgresError;
use std::sync::{Arc, Mutex};
use tracing::{
    Metadata, Subscriber,
    instrument::WithSubscriber,
    span::{Attributes, Id, Record},
};

#[derive(Default)]
struct ReadCounter(Mutex<Vec<&'static str>>);

impl Subscriber for ReadCounter {
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
        let mut statements = self.0.lock().unwrap();
        statements.push(attributes.metadata().name());
        Id::from_u64(statements.len() as u64)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

pub async fn assert_round_trips<T>(
    expected: usize,
    read: impl Future<Output = Result<T, PostgresError>>,
) -> Result<T, PostgresError> {
    let counter = Arc::new(ReadCounter::default());
    let result = read.with_subscriber(Arc::clone(&counter)).await;
    let statements = counter.0.lock().unwrap();
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
    result
}
