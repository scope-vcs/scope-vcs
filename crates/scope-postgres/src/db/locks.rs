use super::entities;
use crate::error::PostgresError;
use sea_orm::{
    ConnectionTrait, EntityTrait, QuerySelect, Set,
    sea_query::{LockType, OnConflict},
};
#[cfg(test)]
use sea_orm::{DatabaseBackend, Statement};

pub async fn acquire_aggregate_lock<C>(
    conn: &C,
    namespace: &str,
    id: &str,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    acquire_aggregate_lock_with_mode(conn, namespace, id, LockType::Update).await
}

/// Request-local writes share the repository guard. Repository policy, membership and
/// lifecycle mutations take its exclusive form before changing authorization facts.
pub(super) async fn acquire_shared_repository_lock<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError> {
    acquire_aggregate_lock_with_mode(conn, "repository", repo_id, LockType::Share).await
}

async fn acquire_aggregate_lock_with_mode<C: ConnectionTrait>(
    conn: &C,
    namespace: &str,
    id: &str,
    mode: LockType,
) -> Result<(), PostgresError> {
    #[cfg(test)]
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT set_config('application_name', $1, true)",
        [format!("scope-test-lock:{namespace}").into()],
    ))
    .await
    .map_err(PostgresError::internal)?;

    let key = format!("{namespace}:{id}");
    entities::metadata_lock::Entity::insert(entities::metadata_lock::ActiveModel {
        key: Set(key.clone()),
    })
    .on_conflict(
        OnConflict::column(entities::metadata_lock::Column::Key)
            .do_nothing()
            .to_owned(),
    )
    .do_nothing()
    .exec(conn)
    .await
    .map_err(PostgresError::internal)?;
    let row = entities::metadata_lock::Entity::find_by_id(key)
        .lock(mode)
        .one(conn)
        .await
        .map_err(PostgresError::internal)?;
    if row.is_none() {
        return Err(PostgresError::internal_message(
            "aggregate lock row disappeared during acquisition",
        ));
    }
    Ok(())
}

/// Observe a waiter blocked by this transaction. The deadline detects hangs, not ordering.
#[cfg(test)]
pub(super) async fn wait_for_transaction_waiter(
    store: &super::MetadataStore,
    blocker_pid: i32,
) -> i32 {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let waiting = store.db.query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT pid FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) LIMIT 1",
                [blocker_pid.into()],
            )).await.unwrap();
            if let Some(waiting) = waiting {
                break waiting.try_get::<i32>("", "pid").unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("expected a waiter blocked by the held transaction")
}

/// Observe the exact schema-scoped advisory lock, so unrelated database work cannot
/// satisfy a test's waiting barrier. The deadline detects hangs, not ordering.
#[cfg(test)]
pub(super) async fn wait_for_advisory_waiter(
    store: &super::MetadataStore,
    namespace: &str,
    repo_id: &str,
) {
    let schema = super::content_fences::current_schema(store.db.as_ref())
        .await
        .unwrap();
    let key = sea_orm::sqlx::postgres::PgAdvisoryLock::new(format!(
        "scope:{namespace}:{schema}:{repo_id}"
    ))
    .key()
    .as_bigint()
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let waiting = store.db.query_one(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT EXISTS (
                    SELECT 1 FROM pg_locks waiter JOIN pg_locks holder
                      USING (locktype, database, classid, objid, objsubid)
                    WHERE waiter.locktype = 'advisory'
                      AND waiter.database = (SELECT oid FROM pg_database WHERE datname = current_database())
                      AND waiter.classid::bigint = $1 AND waiter.objid::bigint = $2
                      AND waiter.objsubid = 1 AND NOT waiter.granted AND holder.granted
                      AND holder.pid = ANY(pg_blocking_pids(waiter.pid))
                ) AS waiting",
                [((key as u64 >> 32) as i64).into(), ((key as u64 & 0xffffffff) as i64).into()],
            )).await.unwrap().unwrap().try_get::<bool>("", "waiting").unwrap();
            if waiting { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("expected a waiter blocked by the held advisory lock");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{MetadataStore, TestDatabaseTarget};
    use sea_orm::TransactionTrait;
    use std::time::Duration;

    #[tokio::test]
    async fn aggregate_locks_serialize_same_key_without_blocking_other_keys() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        // Existing keys ensure contention reaches SELECT FOR UPDATE instead of
        // blocking on insertion of an uncommitted unique key.
        let seeded = store.db.begin().await.unwrap();
        for id in ["owner/one", "owner/two"] {
            acquire_aggregate_lock(&seeded, "repository", id)
                .await
                .unwrap();
        }
        seeded.commit().await.unwrap();

        let held = store.db.begin().await.unwrap();
        let holder_pid: i32 = held
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT pg_backend_pid() AS pid".to_string(),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "pid")
            .unwrap();
        acquire_aggregate_lock(&held, "repository", "owner/one")
            .await
            .unwrap();

        let same_store = store.clone();
        let same = tokio::spawn(async move {
            let tx = same_store.db.begin().await.unwrap();
            acquire_aggregate_lock(&tx, "repository", "owner/one")
                .await
                .unwrap();
            tx.commit().await.unwrap();
        });
        let other_store = store.clone();
        let other = tokio::spawn(async move {
            let tx = other_store.db.begin().await.unwrap();
            acquire_aggregate_lock(&tx, "repository", "owner/two")
                .await
                .unwrap();
            tx.commit().await.unwrap();
        });

        tokio::time::timeout(Duration::from_secs(2), other)
            .await
            .expect("different aggregate key should not block")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let waiting: bool = store
                    .db
                    .query_one(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        "SELECT EXISTS (
                        SELECT 1 FROM pg_stat_activity waiter
                        WHERE waiter.application_name = 'scope-test-lock:repository'
                          AND $1 = ANY(pg_blocking_pids(waiter.pid))
                          AND waiter.query LIKE 'SELECT %scope_metadata_locks%FOR UPDATE%'
                    ) AS waiting",
                        [holder_pid.into()],
                    ))
                    .await
                    .unwrap()
                    .unwrap()
                    .try_get("", "waiting")
                    .unwrap();
                if waiting {
                    break;
                }
                assert!(!same.is_finished(), "same-key writer bypassed the row lock");
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("same-key writer must block on SELECT FOR UPDATE");
        held.commit().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), same)
            .await
            .expect("same aggregate key should proceed after release")
            .unwrap();
    }
}
