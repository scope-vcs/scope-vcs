use crate::{db::requests::tests::postgres_store, error::PostgresErrorKind};

#[tokio::test]
async fn deadlocks_and_serialization_failures_are_told_apart_and_retryable() {
    let store = postgres_store();
    for (sqlstate, kind, retryable) in [
        ("40P01", PostgresErrorKind::Deadlock, true),
        ("40001", PostgresErrorKind::SerializationFailure, true),
        ("22012", PostgresErrorKind::Internal, false),
    ] {
        let error = store
            .admin()
            .execute_for_tests(&format!(
                "DO $$ BEGIN RAISE EXCEPTION 'failure' USING ERRCODE = '{sqlstate}'; END $$"
            ))
            .await
            .unwrap_err();
        assert_eq!(error.kind, kind);
        assert_eq!(error.is_retryable(), retryable);
    }
}
