use super::*;
use sea_orm::sqlx::Connection as _;

const EXIT_CLEANUP_CHILD: &str = "SCOPE_TEST_DATABASE_EXIT_CLEANUP_CHILD";

#[test]
fn database_url_replaces_database_and_preserves_query() {
    assert_eq!(
        database_url(
            "postgres://scope:scope@localhost:5432/scope_test?sslmode=disable",
            "scope_test_db_1"
        )
        .unwrap(),
        "postgres://scope:scope@localhost:5432/scope_test_db_1?sslmode=disable"
    );
}

#[test]
fn template_databases_isolate_targets_and_preserve_live_reopens() {
    let target = TestDatabaseTarget::required().unwrap();
    let first = connect_postgres_test_store(&target).unwrap();
    let other = connect_postgres_test_store(&TestDatabaseTarget::required().unwrap()).unwrap();
    let database = Arc::clone(&first.db);
    run_test_future(async move {
        database
            .execute_unprepared(
                "INSERT INTO scope_users (id, handle, email, email_verified) VALUES ('template_fixture', 'template_fixture', 'template-fixture@scope.test', TRUE)",
            )
            .await
            .unwrap();
    });

    let reopened = connect_postgres_test_store(&target).unwrap();
    assert!(Arc::ptr_eq(&first.db, &reopened.db));
    drop(first);
    run_test_future(async move {
        for (store, expected) in [(reopened, 1_i64), (other, 0_i64)] {
            let row = store
                .db
                .query_one_raw(Statement::from_string(
                    store.db.get_database_backend(),
                    "SELECT current_database() AS database, (SELECT count(*) FROM scope_users WHERE id = 'template_fixture') AS count".to_owned(),
                ))
                .await
                .unwrap()
                .unwrap();
            assert!(
                row.try_get::<String>("", "database")
                    .unwrap()
                    .starts_with("scope_test_db_")
            );
            assert_eq!(row.try_get::<i64>("", "count").unwrap(), expected);
            crate::migrations::assert_exact_state(store.db.as_ref())
                .await
                .unwrap();
        }
    });

    let fresh = connect_postgres_test_store(&target).unwrap();
    run_test_future(async move {
        let count = fresh
            .db
            .query_one_raw(Statement::from_string(
                fresh.db.get_database_backend(),
                "SELECT count(*) AS count FROM scope_users".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "count")
            .unwrap();
        assert_eq!(count, 0);
    });
}

#[test]
fn dropped_stores_release_their_connections_while_database_drops_are_queued() {
    let target = TestDatabaseTarget::required().unwrap();
    let store = connect_postgres_test_store(&target).unwrap();
    let database = Arc::clone(&store.db);
    let database_name = run_test_future(async move {
        let pool = database.get_postgres_connection_pool();
        let mut connections = Vec::new();
        for _ in 0..3 {
            connections.push(pool.acquire().await.unwrap());
        }
        drop(connections);
        database
            .query_one_raw(Statement::from_string(
                database.get_database_backend(),
                "SELECT current_database() AS name".to_owned(),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<String>("", "name")
            .unwrap()
    });
    let queued_drops =
        run_test_future(async { DATABASE_DROPS.acquire_many(DATABASE_DROP_CONCURRENCY).await })
            .unwrap();

    drop(store);
    let admin_url = target.database_url.clone();
    let name = database_name.clone();
    let remaining = run_test_future(async move {
        let admin = Database::connect(&admin_url).await.unwrap();
        let deadline = tokio::time::Instant::now() + BACKEND_STARTUP_ALLOWANCE;
        let remaining = loop {
            let remaining = admin
                .query_one_raw(Statement::from_sql_and_values(
                    admin.get_database_backend(),
                    "SELECT count(*)::bigint AS count FROM pg_stat_activity WHERE datname = $1",
                    [name.clone().into()],
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get::<i64>("", "count")
                .unwrap();
            if remaining == 0 || tokio::time::Instant::now() >= deadline {
                break remaining;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        admin.close().await.unwrap();
        remaining
    });
    drop(queued_drops);

    assert_eq!(
        remaining, 0,
        "{database_name} kept connections while its drop waited behind other database drops"
    );
}

#[test]
fn process_exit_removes_only_its_registered_databases() {
    if let Some(mode) = std::env::var_os(EXIT_CLEANUP_CHILD) {
        let target = TestDatabaseTarget::required().unwrap();
        let store = connect_postgres_test_store(&target).unwrap();
        if mode == "orphaned-lease" {
            let name = orphaned_client_with_idle_connection(&store, &target);
            drop(store);
            let admin_url = target.database_url.clone();
            let database_name = name.clone();
            let (absent, slot_freed) = run_test_future(async move {
                let admin = Database::connect(&admin_url).await.unwrap();
                let deadline = tokio::time::Instant::now() + EXIT_CLEANUP_TIMEOUT;
                let absent = loop {
                    let row = admin.query_one_raw(Statement::from_sql_and_values(
                        admin.get_database_backend(),
                        "SELECT NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1) AS absent",
                        [database_name.clone().into()],
                    )).await.unwrap().unwrap();
                    let absent = row.try_get::<bool>("", "absent").unwrap();
                    if absent || tokio::time::Instant::now() >= deadline {
                        break absent;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                };
                admin.close().await.unwrap();
                let slot_freed = loop {
                    let freed =
                        OUTSTANDING_DATABASES.available_permits() == OUTSTANDING_DATABASE_LIMIT;
                    if freed || tokio::time::Instant::now() >= deadline {
                        break freed;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                };
                (absent, slot_freed)
            });
            unregister_process_database(&target.database_url, &name);
            assert!(absent, "normal lease cleanup left owned database {name}");
            assert!(
                slot_freed,
                "dropping {name} kept its outstanding database slot while a pooled connection was never returned"
            );
            return;
        }
        if mode == "orphaned-client" {
            let name = orphaned_client_with_idle_connection(&store, &target);
            cleanup_process_databases();
            let admin_url = target.database_url.clone();
            let database_name = name.clone();
            let absent = run_test_future(async move {
                let admin = Database::connect(&admin_url).await.unwrap();
                let row = admin.query_one_raw(Statement::from_sql_and_values(
                    admin.get_database_backend(),
                    "SELECT NOT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1) AS absent",
                    [database_name.into()],
                )).await.unwrap().unwrap();
                let absent = row.try_get::<bool>("", "absent").unwrap();
                admin.close().await.unwrap();
                absent
            });
            unregister_process_database(&target.database_url, &name);
            assert!(
                absent,
                "process cleanup left owned database {name} after its checked-out backend exited"
            );
            assert!(store.db.get_postgres_connection_pool().is_closed());
        }
        if mode == "pool-close" {
            cleanup_process_databases();
            assert!(
                store.db.get_postgres_connection_pool().is_closed(),
                "process cleanup left the registered database pool open"
            );
        }
        std::mem::forget(store);
        return;
    }

    let target = TestDatabaseTarget::required().unwrap();
    let (_, pool_child) = cleanup_child(&target, "pool-close");
    assert!(
        pool_child.status.success(),
        "pool cleanup child failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&pool_child.stdout),
        String::from_utf8_lossy(&pool_child.stderr)
    );
    let (_, orphan_child) = cleanup_child(&target, "orphaned-client");
    assert!(
        orphan_child.status.success(),
        "orphaned client cleanup child failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&orphan_child.stdout),
        String::from_utf8_lossy(&orphan_child.stderr)
    );
    let (_, lease_child) = cleanup_child(&target, "orphaned-lease");
    assert!(
        lease_child.status.success(),
        "normal lease cleanup child failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&lease_child.stdout),
        String::from_utf8_lossy(&lease_child.stderr)
    );
    let external_database = unique_database_name("external_process");
    let admin_url = target.database_url.clone();
    let external_name = external_database.clone();
    let mut external_connection = run_test_future(async move {
        let admin = Database::connect(&admin_url).await.unwrap();
        execute(
            &admin,
            format!(
                "CREATE DATABASE {} TEMPLATE template0",
                quote_pg_ident(&external_name)
            ),
        )
        .await
        .unwrap();
        admin.close().await.unwrap();
        sea_orm::sqlx::PgConnection::connect(&database_url(&admin_url, &external_name).unwrap())
            .await
            .unwrap()
    });

    let (child_id, child) = cleanup_child(&target, "exit");
    assert!(
        child.status.success(),
        "cleanup child failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );

    let external_value = run_test_future(async move {
        let value: (i32,) = sea_orm::sqlx::query_as("SELECT 1")
            .fetch_one(&mut external_connection)
            .await
            .unwrap();
        external_connection.close().await.unwrap();
        value.0
    });
    assert_eq!(external_value, 1);

    let admin_url = target.database_url.clone();
    let external_name = external_database.clone();
    let databases = run_test_future(async move {
        let admin = Database::connect(&admin_url).await.unwrap();
        let deadline = tokio::time::Instant::now() + EXIT_CLEANUP_TIMEOUT;
        let databases = loop {
            let databases = admin
                .query_all_raw(Statement::from_string(
                    admin.get_database_backend(),
                    "SELECT datname FROM pg_database".to_owned(),
                ))
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.try_get::<String>("", "datname").unwrap())
                .collect::<Vec<_>>();
            let child_database_remains = databases.iter().any(|name| {
                name.starts_with(&format!("scope_test_db_{child_id}_"))
                    || name.starts_with(&format!("scope_test_template_{child_id}_"))
            });
            if !child_database_remains || tokio::time::Instant::now() >= deadline {
                break databases;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        drop_once_backends_are_terminable(&admin, &external_name)
            .await
            .unwrap();
        admin.close().await.unwrap();
        databases
    });
    assert!(databases.contains(&external_database));
    assert!(
        databases.iter().all(
            |name| !name.starts_with(&format!("scope_test_db_{child_id}_"))
                && !name.starts_with(&format!("scope_test_template_{child_id}_"))
        ),
        "cleanup child left a registered database behind: {databases:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
}

fn orphaned_client_with_idle_connection(
    store: &MetadataStore,
    target: &TestDatabaseTarget,
) -> String {
    let database = Arc::clone(&store.db);
    let admin_url = target.database_url.clone();
    let (name, terminated, idle_clients) = run_test_future(async move {
        let pool = database.get_postgres_connection_pool();
        let mut orphan = pool.acquire().await.unwrap();
        let (name, pid) = sea_orm::sqlx::query_as::<_, (String, i32)>(
            "SELECT current_database(), pg_backend_pid()",
        )
        .fetch_one(&mut *orphan)
        .await
        .unwrap();
        let mut idle = pool.acquire().await.unwrap();
        idle.return_to_pool().await;
        drop(idle);
        let admin = Database::connect(&admin_url).await.unwrap();
        let terminated = admin
            .query_one_raw(Statement::from_sql_and_values(
                admin.get_database_backend(),
                "SELECT pg_terminate_backend($1) AS terminated",
                [pid.into()],
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<bool>("", "terminated")
            .unwrap();
        let idle_clients = admin.query_one_raw(Statement::from_sql_and_values(
            admin.get_database_backend(),
            "SELECT count(*)::bigint AS count FROM pg_stat_activity
             WHERE datname = $1 AND pid <> $2 AND usename = current_user AND backend_type = 'client backend'",
            [name.clone().into(), pid.into()],
        )).await.unwrap().unwrap().try_get::<i64>("", "count").unwrap();
        admin.close().await.unwrap();
        std::mem::forget(orphan);
        (name, terminated, idle_clients)
    });
    assert!(
        terminated,
        "failed to terminate only the borrowed backend for {name}"
    );
    assert_eq!(
        idle_clients, 1,
        "regression requires one live idle client for {name}"
    );
    name
}

fn cleanup_child(target: &TestDatabaseTarget, mode: &str) -> (u32, std::process::Output) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .arg("--exact")
        .arg(
            "db::test_support::database_templates::tests::process_exit_removes_only_its_registered_databases",
        )
        .env(EXIT_CLEANUP_CHILD, mode)
        .env("SCOPE_TEST_DATABASE_URL", &target.database_url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let child = command.spawn().unwrap();
    let id = child.id();
    (id, child.wait_with_output().unwrap())
}
