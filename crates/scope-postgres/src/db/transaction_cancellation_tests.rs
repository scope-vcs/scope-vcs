use super::{
    GitHubCheckCommit,
    data_scope::{DataScope, ScopedDatabase, SystemReason},
    requests::tests::postgres_store,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use sqlx::{Connection, postgres::PgPoolOptions};
use std::{future::poll_fn, sync::Arc, task::Poll};

#[tokio::test]
async fn cancelled_begin_cannot_roll_back_the_next_borrowers_check_read() {
    let mut store = postgres_store();
    let observer = Arc::clone(&store.db);
    let schema = observer
        .fixture()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT current_schema() AS name".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "name")
        .unwrap();
    let options = observer.pool_for_tests().connect_options();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with((*options).clone().options([("search_path", schema)]))
        .await
        .unwrap();
    store.db = Arc::new(ScopedDatabase::from_pool(pool.clone()));
    let commit = GitHubCheckCommit {
        repo_id: "owner/repo".into(),
        github_repository_id: 42,
        commit_oid: "a".repeat(40),
    };
    let requests = store.requests();
    assert_eq!(requests.start_github_check_read(&commit).await.unwrap(), 1);

    let blocker = observer.begin(DataScope::NoRepository).await.unwrap();
    blocker
        .execute_unprepared(
            "SELECT pg_advisory_xact_lock(hashtextextended(current_database(), 73))",
        )
        .await
        .unwrap();
    let mut connection = pool.acquire().await.unwrap();
    let original_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    let mut begin = Box::pin(connection.begin_with(
        "BEGIN; SELECT pg_advisory_xact_lock(hashtextextended(current_database(), 73))",
    ));
    assert!(poll_fn(|cx| Poll::Ready(begin.as_mut().poll(cx).is_pending())).await);
    drop(begin);
    drop(connection);
    blocker.commit().await.unwrap();

    let reused_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(reused_pid, original_pid);
    let read = requests.start_github_check_read(&commit).await.unwrap();
    assert_eq!(read, 2);
    let visible_read = observer
        .fixture()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT started_reads FROM scope_github_check_refreshes".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "started_reads")
        .unwrap();
    assert_eq!(visible_read, 2, "a returned read number must be committed");

    let snapshot = store
        .db
        .begin_read_snapshot(DataScope::System(SystemReason::Fixture))
        .await
        .unwrap();
    snapshot.commit().await.unwrap();
    assert!(
        requests
            .apply_github_check_read(&commit, read, 100, &[])
            .await
            .unwrap()
    );
    pool.close().await;
}
