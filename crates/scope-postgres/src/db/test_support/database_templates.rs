use super::*;
use anyhow::Context as _;
use std::{
    collections::HashMap,
    sync::{Mutex, Weak},
    time::Duration,
};

const TEMPLATE_SCHEMA: &str = "scope_test_template";
const EXIT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(90);

type ProcessDatabases = Mutex<HashMap<String, HashMap<String, Weak<DatabaseConnection>>>>;
static PROCESS_DATABASES: OnceLock<ProcessDatabases> = OnceLock::new();
static EXIT_HOOK: OnceLock<Result<(), i32>> = OnceLock::new();

pub(super) async fn connect_store(target: &TestDatabaseTarget) -> anyhow::Result<MetadataStore> {
    let mut active_database = target.active_database.lock().await;
    let lease = match active_database.upgrade() {
        Some(lease) => lease,
        None => {
            let lease = clone_test_database(target).await?;
            *active_database = Arc::downgrade(&lease);
            lease
        }
    };

    Ok(MetadataStore {
        db: Arc::clone(&lease.database),
        postgres_database_url: Some(Arc::from(lease.database_url.clone())),
        _test_schema: Some(lease),
    })
}

async fn clone_test_database(target: &TestDatabaseTarget) -> anyhow::Result<Arc<TestSchemaLease>> {
    let template = template_database(&target.database_url).await?;
    let database_name = unique_database_name("db");
    let database_url = database_url(&target.database_url, &database_name)?;
    create_database(&target.database_url, &database_name, &template).await?;

    let mut options = ConnectOptions::new(database_url.clone());
    options
        .max_connections(8)
        .min_connections(1)
        .set_schema_search_path(TEMPLATE_SCHEMA);
    let database = match Database::connect(options).await {
        Ok(database) => Arc::new(database),
        Err(error) => {
            drop_database(&target.database_url, &database_name).await;
            return Err(error.into());
        }
    };

    register_process_database(
        &target.database_url,
        &database_name,
        Arc::downgrade(&database),
    )?;

    Ok(Arc::new(TestSchemaLease {
        database,
        database_url,
        cleanup: TestDatabaseCleanup::Database {
            admin_url: target.database_url.clone(),
            database_name,
        },
    }))
}

async fn template_database(admin_url: &str) -> anyhow::Result<String> {
    type Templates = Mutex<HashMap<String, Arc<tokio::sync::OnceCell<String>>>>;
    static TEMPLATES: OnceLock<Templates> = OnceLock::new();

    let template = TEMPLATES
        .get_or_init(Mutex::default)
        .lock()
        .expect("test database template registry should not be poisoned")
        .entry(admin_url.to_owned())
        .or_default()
        .clone();

    template
        .get_or_try_init(|| prepare_template_database(admin_url))
        .await
        .cloned()
}

async fn prepare_template_database(admin_url: &str) -> anyhow::Result<String> {
    let database_name = unique_database_name("template");
    create_database(admin_url, &database_name, "template0").await?;

    let result = async {
        let database_url = database_url(admin_url, &database_name)?;
        let database = Database::connect(&database_url).await?;
        execute(
            &database,
            format!("CREATE SCHEMA {}", quote_pg_ident(TEMPLATE_SCHEMA)),
        )
        .await?;
        database.close().await?;

        let mut options = ConnectOptions::new(database_url);
        options
            .max_connections(8)
            .min_connections(1)
            .set_schema_search_path(TEMPLATE_SCHEMA);
        let database = Database::connect(options).await?;
        let migration_result =
            crate::migrations::apply_in_maintenance(&database, Default::default()).await;
        database.close().await?;
        migration_result?;
        Ok::<_, anyhow::Error>(database_name.clone())
    }
    .await;

    if result.is_err() {
        drop_database(admin_url, &database_name).await;
    }
    result
}

async fn create_database(
    admin_url: &str,
    database_name: &str,
    template_name: &str,
) -> anyhow::Result<()> {
    register_process_database(admin_url, database_name, Weak::new())?;
    let admin = match Database::connect(admin_url).await {
        Ok(admin) => admin,
        Err(error) => {
            unregister_process_database(admin_url, database_name);
            return Err(error.into());
        }
    };
    let create_result = execute(
        &admin,
        format!(
            "CREATE DATABASE {} TEMPLATE {}",
            quote_pg_ident(database_name),
            quote_pg_ident(template_name)
        ),
    )
    .await
    .with_context(|| {
        format!(
            "creating PostgreSQL test database {database_name}; the SCOPE_TEST_DATABASE_URL role must have CREATEDB"
        )
    });
    let close_result = admin.close().await;
    if let Err(error) = create_result {
        let _ = try_drop_database(admin_url, database_name).await;
        return Err(error);
    }
    if let Err(error) = close_result {
        let _ = try_drop_database(admin_url, database_name).await;
        return Err(error.into());
    }
    Ok(())
}

fn register_process_database(
    admin_url: &str,
    database_name: &str,
    database: Weak<DatabaseConnection>,
) -> anyhow::Result<()> {
    match EXIT_HOOK.get_or_init(|| {
        let result = unsafe { libc::atexit(cleanup_process_databases_at_exit) };
        (result == 0).then_some(()).ok_or(result)
    }) {
        Ok(()) => {}
        Err(code) => anyhow::bail!("registering PostgreSQL test cleanup hook failed with {code}"),
    }
    PROCESS_DATABASES
        .get_or_init(Mutex::default)
        .lock()
        .expect("test database registry should not be poisoned")
        .entry(admin_url.to_owned())
        .or_default()
        .insert(database_name.to_owned(), database);
    Ok(())
}

fn unregister_process_database(admin_url: &str, database_name: &str) {
    let Some(databases) = PROCESS_DATABASES.get() else {
        return;
    };
    let mut databases = databases
        .lock()
        .expect("test database registry should not be poisoned");
    let Some(names) = databases.get_mut(admin_url) else {
        return;
    };
    names.remove(database_name);
    if names.is_empty() {
        databases.remove(admin_url);
    }
}

extern "C" fn cleanup_process_databases_at_exit() {
    if std::panic::catch_unwind(cleanup_process_databases).is_err() {
        eprintln!("PostgreSQL test database process cleanup panicked");
    }
}

fn cleanup_process_databases() {
    let databases = registered_process_databases();
    if databases.is_empty() {
        return;
    }
    let mut servers = databases
        .iter()
        .map(|(admin_url, _, _)| admin_url.clone())
        .collect::<Vec<_>>();
    servers.sort_unstable();
    servers.dedup();

    let cleanup = run_test_future(async move {
        tokio::time::timeout(EXIT_CLEANUP_TIMEOUT, async move {
            let mut cleanups = tokio::task::JoinSet::new();
            for (admin_url, database_name, database) in databases {
                cleanups.spawn(async move {
                    let result = async {
                        if let Some(database) = database.upgrade() {
                            drop(database.get_postgres_connection_pool().close());
                        }
                        try_drop_database(&admin_url, &database_name).await
                    }
                    .await;
                    (database_name, result)
                });
            }
            let mut failures = 0;
            while let Some(cleanup) = cleanups.join_next().await {
                match cleanup {
                    Ok((_, Ok(()))) => {}
                    Ok((name, Err(error))) => {
                        failures += 1;
                        eprintln!("test database cleanup failed for {name}: {error}");
                    }
                    Err(error) => {
                        failures += 1;
                        eprintln!("test database cleanup task failed: {error}");
                    }
                }
            }
            failures
        })
        .await
    });
    match cleanup {
        Ok(0) => {}
        Ok(failures) => {
            eprintln!("PostgreSQL test database process cleanup failed for {failures} tasks");
            return;
        }
        Err(_) => {
            let remaining = registered_process_databases();
            let names = remaining
                .iter()
                .map(|(_, name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!(
                "PostgreSQL test database cleanup exceeded {} seconds with {} process-owned databases still registered: {names}. A later test process sweeps them once they are an hour old",
                EXIT_CLEANUP_TIMEOUT.as_secs(),
                remaining.len()
            );
            return;
        }
    }
    run_test_future(async move {
        let deadline = tokio::time::Instant::now() + stale_cleanup::SWEEP_BUDGET;
        for admin_url in servers {
            if let Err(error) = stale_cleanup::sweep(&admin_url, deadline).await {
                eprintln!("sweeping abandoned PostgreSQL test state failed: {error}");
            }
        }
    });
}

fn registered_process_databases() -> Vec<(String, String, Weak<DatabaseConnection>)> {
    let Some(databases) = PROCESS_DATABASES.get() else {
        return Vec::new();
    };
    databases
        .lock()
        .expect("test database registry should not be poisoned")
        .iter()
        .flat_map(|(admin_url, names)| {
            names
                .iter()
                .map(|(name, database)| (admin_url.clone(), name.clone(), database.clone()))
        })
        .collect()
}

fn database_url(admin_url: &str, database_name: &str) -> anyhow::Result<String> {
    let (base, query) = admin_url.split_once('?').unwrap_or((admin_url, ""));
    let (authority, _) = base
        .rsplit_once('/')
        .ok_or_else(|| anyhow::anyhow!("SCOPE_TEST_DATABASE_URL must include a database name"))?;
    Ok(format!(
        "{authority}/{database_name}{}{query}",
        if query.is_empty() { "" } else { "?" }
    ))
}

async fn execute(database: &DatabaseConnection, sql: String) -> anyhow::Result<()> {
    database
        .execute_raw(Statement::from_string(database.get_database_backend(), sql))
        .await?;
    Ok(())
}

pub(super) async fn drop_database(admin_url: &str, database_name: &str) {
    if let Err(error) = try_drop_database(admin_url, database_name).await {
        eprintln!("test database cleanup failed for {database_name}: {error}");
    }
}

async fn try_drop_database(admin_url: &str, database_name: &str) -> anyhow::Result<()> {
    static CLEANUPS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
    let _permit = CLEANUPS
        .acquire()
        .await
        .expect("test database cleanup semaphore should stay open");
    let admin = Database::connect(admin_url).await?;
    let drop_result = async {
        admin
            .query_all_raw(Statement::from_sql_and_values(
                admin.get_database_backend(),
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity
                 WHERE datname = $1 AND usename = current_user
                 AND backend_type = 'client backend' AND pid <> pg_backend_pid()",
                [database_name.into()],
            ))
            .await?;
        execute(
            &admin,
            format!("DROP DATABASE IF EXISTS {}", quote_pg_ident(database_name)),
        )
        .await
    }
    .await;
    let close_result = admin.close().await;
    drop_result?;
    close_result?;
    unregister_process_database(admin_url, database_name);
    Ok(())
}

fn unique_database_name(kind: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after UNIX epoch")
        .as_nanos();
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "scope_test_{kind}_{}_{}_{}",
        std::process::id(),
        nanos,
        sequence
    )
}

#[cfg(test)]
mod tests {
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
    fn process_exit_removes_only_its_registered_databases() {
        if let Some(mode) = std::env::var_os(EXIT_CLEANUP_CHILD) {
            let target = TestDatabaseTarget::required().unwrap();
            let store = connect_postgres_test_store(&target).unwrap();
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
            execute(
                &admin,
                format!(
                    "DROP DATABASE {} WITH (FORCE)",
                    quote_pg_ident(&external_name)
                ),
            )
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
}
