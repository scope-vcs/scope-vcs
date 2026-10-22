use super::*;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use std::time::Duration;

async fn wait_for_two_waiters(store: &MetadataStore, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let count: i64 = store.db.query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "WITH RECURSIVE blocked(pid) AS (SELECT pid FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) UNION SELECT activity.pid FROM pg_stat_activity activity JOIN blocked ON blocked.pid = ANY(pg_blocking_pids(activity.pid))) SELECT count(*) AS count FROM blocked",
                [blocker.into()],
            )).await.unwrap().unwrap().try_get("", "count").unwrap();
            if count >= 2 {
                return;
            }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
}

#[tokio::test]
async fn concurrent_retries_of_one_revision_context_return_the_same_receipt() {
    let store = crate::db::requests::tests::postgres_store();
    let parent = start(&store, "parent", main_base(), 2).await;
    let child = start(&store, "child", main_base(), 6).await;
    let base = pin(&store, &parent).await;
    let facts = selected_parent_facts(&store, &child, &base).await;
    let context = context_for(&store, &child, base.clone(), facts.as_ref(), "reparent").await;
    let mut publication = input(&child, &child.head_oid, "reparent_event", 10);
    publication.new_base = Some(base);
    let held = store.db.begin().await.unwrap();
    crate::db::locks::acquire_shared_repository_lock(&held, &child.repo_id)
        .await
        .unwrap();
    crate::db::acquire_aggregate_lock(&held, "request", &child.id)
        .await
        .unwrap();
    let blocker = crate::db::locks::backend_pid(&held).await;
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let store = store.clone();
        let publication = publication.clone();
        let context = context.clone();
        let facts = facts.clone();
        tasks.push(tokio::spawn(async move {
            store
                .requests()
                .record_request_revision_context(publication, context, facts, Vec::new())
                .await
        }));
    }
    wait_for_two_waiters(&store, blocker).await;
    held.commit().await.unwrap();
    let first = tasks.remove(0).await.unwrap();
    let second = tasks.remove(0).await.unwrap();
    assert!(
        first.is_ok() && second.is_ok(),
        "first={first:?}; second={second:?}"
    );
    assert_eq!(first.unwrap().revision.id, second.unwrap().revision.id);
}

#[tokio::test]
async fn a_reparent_while_a_stack_waits_for_its_locks_cannot_deadlock() {
    let store = crate::db::requests::tests::postgres_store();
    let root = start(&store, "b-root", main_base(), 2).await;
    let middle = start(&store, "c-middle", pin(&store, &root).await, 6).await;
    let child = start(&store, "a-child", main_base(), 10).await;
    let base = pin(&store, &middle).await;
    let facts = selected_parent_facts(&store, &child, &base).await;
    let mut publication = input(&child, &child.head_oid, "reparent_event", 14);
    publication.new_base = Some(base);
    let held = store.db.begin().await.unwrap();
    crate::db::locks::acquire_shared_repository_lock(&held, &root.repo_id)
        .await
        .unwrap();
    crate::db::acquire_aggregate_lock(&held, "request", &root.id)
        .await
        .unwrap();
    let blocker = crate::db::locks::backend_pid(&held).await;
    let moving_store = store.clone();
    let moving =
        tokio::spawn(async move { publish(&moving_store, &child, publication, facts, true).await });
    crate::db::locks::wait_for_transaction_waiter(&store, blocker).await;
    let graph_store = store.clone();
    let graph_root = root.clone();
    let graph = tokio::spawn(async move {
        let tx = graph_store.db.begin().await.unwrap();
        crate::db::locks::acquire_shared_repository_lock(&tx, &graph_root.repo_id)
            .await
            .unwrap();
        lock_graph(&tx, &graph_root, None).await.unwrap();
        tx
    });
    wait_for_two_waiters(&store, blocker).await;
    held.commit().await.unwrap();
    let moved = moving.await.unwrap().unwrap().request;
    let stale = graph.await.unwrap();
    let stale_pid = crate::db::locks::backend_pid(&stale).await;
    let revision_facts = parent_facts(&store, &moved).await;
    let revising_store = store.clone();
    let revising = tokio::spawn(async move {
        revising_store
            .requests()
            .record_request_revision_with_stack(
                input(&moved, "child_next", "child_next_event", 16),
                moved.base_version,
                revision_facts,
                Vec::new(),
            )
            .await
    });
    crate::db::locks::wait_for_transaction_waiter(&store, stale_pid).await;
    let refresh = refresh_children(&stale, &root, &[fact(&root.head_oid, &middle, true)], 17).await;
    stale.rollback().await.unwrap();
    let revision = revising.await.unwrap();
    assert!(
        refresh.is_ok() && revision.is_ok(),
        "refresh={refresh:?}; revision={revision:?}"
    );
}
