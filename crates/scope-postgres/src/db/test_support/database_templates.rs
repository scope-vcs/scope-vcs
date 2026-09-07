use super::*;
use anyhow::Context as _;
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Duration,
};

const TEMPLATE_SCHEMA: &str = "scope_test_template";
const EXIT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);

type ProcessDatabases = Mutex<HashMap<String, HashSet<String>>>;
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
        let migration_result = crate::migrations::apply_in_maintenance(&database).await;
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
    register_process_database(admin_url, database_name)?;
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

fn register_process_database(admin_url: &str, database_name: &str) -> anyhow::Result<()> {
    match EXIT_HOOK.get_or_init(|| {
        // The callback drains only names registered by this process. It runs on
        // normal process exit while the dedicated test runtime is still alive.
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
        .insert(database_name.to_owned());
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
    // Panics cannot cross an FFI callback. Cleanup failures are reported by the
    // individual drop operation and must never abort an otherwise successful test run.
    let _ = std::panic::catch_unwind(cleanup_process_databases);
}

fn cleanup_process_databases() {
    let databases = registered_process_databases();
    if databases.is_empty() {
        return;
    }

    let completed = run_test_future(async move {
        tokio::time::timeout(EXIT_CLEANUP_TIMEOUT, async move {
            let mut cleanups = tokio::task::JoinSet::new();
            for (admin_url, database_name) in databases {
                cleanups.spawn(async move {
                    drop_database(&admin_url, &database_name).await;
                });
            }
            while cleanups.join_next().await.is_some() {}
        })
        .await
        .is_ok()
    });
    if !completed {
        let remaining = registered_process_databases();
        let names = remaining
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "PostgreSQL test database cleanup exceeded {} seconds with {} process-owned databases still registered: {names}. Drops waiting for a cleanup permit or a backlogged or unreachable server may require manual cleanup",
            EXIT_CLEANUP_TIMEOUT.as_secs(),
            remaining.len()
        );
    }
}

fn registered_process_databases() -> Vec<(String, String)> {
    let Some(databases) = PROCESS_DATABASES.get() else {
        return Vec::new();
    };
    databases
        .lock()
        .expect("test database registry should not be poisoned")
        .iter()
        .flat_map(|(admin_url, names)| names.iter().map(|name| (admin_url.clone(), name.clone())))
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
        .execute(Statement::from_string(database.get_database_backend(), sql))
        .await?;
    Ok(())
}

pub(super) async fn drop_database(admin_url: &str, database_name: &str) {
    if let Err(error) = try_drop_database(admin_url, database_name).await {
        eprintln!("test database cleanup failed for {database_name}: {error}");
    }
}

async fn try_drop_database(admin_url: &str, database_name: &str) -> anyhow::Result<()> {
    // Keep slow drops from consuming every server connection while later tests
    // are already opening their database pools.
    static CLEANUPS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
    let _permit = CLEANUPS
        .acquire()
        .await
        .expect("test database cleanup semaphore should stay open");
    let admin = Database::connect(admin_url).await?;
    let drop_result = execute(
        &admin,
        format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            quote_pg_ident(database_name)
        ),
    )
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
                    .query_one(Statement::from_string(
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

        // Once every live store is gone, this target receives a fresh database
        // even if asynchronous cleanup of its prior database is still running.
        let fresh = connect_postgres_test_store(&target).unwrap();
        run_test_future(async move {
            let count = fresh
                .db
                .query_one(Statement::from_string(
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
        if std::env::var_os(EXIT_CLEANUP_CHILD).is_some() {
            let store =
                connect_postgres_test_store(&TestDatabaseTarget::required().unwrap()).unwrap();
            // Leave both the live clone and its template for the exit owner. A
            // force drop must close the clone's still-open pool.
            std::mem::forget(store);
            return;
        }

        let target = TestDatabaseTarget::required().unwrap();
        let external_database = unique_database_name("external_process");
        let admin_url = target.database_url.clone();
        let external_name = external_database.clone();
        run_test_future(async move {
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
        });

        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg(
                "db::test_support::database_templates::tests::process_exit_removes_only_its_registered_databases",
            )
            .env(EXIT_CLEANUP_CHILD, "1")
            .env("SCOPE_TEST_DATABASE_URL", &target.database_url)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let child = command.spawn().unwrap();
        let child_id = child.id();
        let child = child.wait_with_output().unwrap();
        assert!(
            child.status.success(),
            "cleanup child failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );

        let admin_url = target.database_url.clone();
        let external_name = external_database.clone();
        let databases = run_test_future(async move {
            let admin = Database::connect(&admin_url).await.unwrap();
            let deadline = tokio::time::Instant::now() + EXIT_CLEANUP_TIMEOUT;
            let databases = loop {
                let databases = admin
                    .query_all(Statement::from_string(
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
            "cleanup child left a registered database behind: {databases:?}"
        );
    }
}
