//! Temporary, opt-in staging experiment. Templates live only on the disposable test server.
use super::*;
use std::{collections::HashMap, io::Write, sync::Mutex, time::Instant};

const TEMPLATE_SCHEMA: &str = "scope_test_template";

pub(super) fn template_enabled() -> anyhow::Result<bool> {
    match std::env::var("SCOPE_DB_EXPERIMENT_MODE").as_deref() {
        Ok("template") => Ok(true),
        Ok("baseline") | Err(std::env::VarError::NotPresent) => Ok(false),
        _ => anyhow::bail!("SCOPE_DB_EXPERIMENT_MODE must be baseline or template"),
    }
}

pub(super) struct Timer(Option<(Instant, String, String, String)>);

impl Timer {
    pub(super) fn new(phase: &str, target: &str) -> Self {
        Self(
            std::env::var("SCOPE_DB_EXPERIMENT_METRICS")
                .ok()
                .map(|path| (Instant::now(), path, phase.to_owned(), target.to_owned())),
        )
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let Some((start, path, phase, target)) = &self.0 else {
            return;
        };
        let mut line = serde_json::to_vec(&serde_json::json!({
            "pid": std::process::id(),
            "mode": std::env::var("SCOPE_DB_EXPERIMENT_MODE").unwrap_or_default(),
            "phase": phase,
            "target": target,
            "elapsed_ms": start.elapsed().as_secs_f64() * 1000.0,
        }))
        .expect("experiment metrics serialize");
        line.push(b'\n');
        // One O_APPEND write keeps concurrent test processes' JSONL records together.
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open experiment metrics");
        assert_eq!(
            file.write(&line).expect("write experiment metrics"),
            line.len()
        );
    }
}

fn database_url(admin_url: &str, database: &str) -> anyhow::Result<String> {
    let (base, query) = admin_url.split_once('?').unwrap_or((admin_url, ""));
    let (authority, _) = base
        .rsplit_once('/')
        .ok_or_else(|| anyhow::anyhow!("database URL needs a path"))?;
    Ok(format!(
        "{authority}/{database}{}{query}",
        if query.is_empty() { "" } else { "?" }
    ))
}

fn experiment_name() -> String {
    unique_test_schema_name().replacen("scope_test_", "scope_test_experiment_", 1)
}

async fn execute(db: &DatabaseConnection, sql: String) -> anyhow::Result<()> {
    db.execute(Statement::from_string(db.get_database_backend(), sql))
        .await?;
    Ok(())
}

async fn template_database(admin_url: &str) -> anyhow::Result<String> {
    type Templates = Mutex<HashMap<String, Arc<tokio::sync::OnceCell<String>>>>;
    static TEMPLATES: OnceLock<Templates> = OnceLock::new();
    let cell = TEMPLATES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .entry(admin_url.to_owned())
        .or_default()
        .clone();
    cell.get_or_try_init(|| async {
        let name = experiment_name();
        let _timer = Timer::new("template_prepare", &name);
        let admin = Database::connect(admin_url).await?;
        execute(
            &admin,
            format!(
                "CREATE DATABASE {} TEMPLATE template0",
                quote_pg_ident(&name)
            ),
        )
        .await?;
        admin.close().await?;
        let result = async {
            let db = Database::connect(database_url(admin_url, &name)?).await?;
            execute(
                &db,
                format!("CREATE SCHEMA {}", quote_pg_ident(TEMPLATE_SCHEMA)),
            )
            .await?;
            db.close().await?;
            let mut options = ConnectOptions::new(database_url(admin_url, &name)?);
            options
                .max_connections(8)
                .min_connections(1)
                .set_schema_search_path(TEMPLATE_SCHEMA);
            let db = Database::connect(options).await?;
            let timer = Timer::new("template_migrate", &name);
            let result = crate::migrations::apply_in_maintenance(&db).await;
            drop(timer);
            db.close().await?;
            result?;
            Ok::<_, anyhow::Error>(name.clone())
        }
        .await;
        if result.is_err() {
            drop_database(admin_url, &name).await;
        }
        result
    })
    .await
    .cloned()
}

pub(super) async fn connect_template_store(
    target: &TestDatabaseTarget,
) -> anyhow::Result<MetadataStore> {
    let _timer = Timer::new("store_setup", &target.schema_name);
    let mut current = target.experiment_lease.lock().await;
    let lease = if let Some(lease) = current.upgrade() {
        lease
    } else {
        let template = template_database(&target.database_url).await?;
        // A new generation gets a new name, so prior asynchronous cleanup cannot delete it.
        let name = experiment_name();
        let url = database_url(&target.database_url, &name)?;
        let timer = Timer::new("admin_connect", &target.schema_name);
        let admin = Database::connect(&target.database_url).await?;
        drop(timer);
        let timer = Timer::new("database_clone", &target.schema_name);
        execute(
            &admin,
            format!(
                "CREATE DATABASE {} TEMPLATE {}",
                quote_pg_ident(&name),
                quote_pg_ident(&template)
            ),
        )
        .await?;
        drop(timer);
        admin.close().await?;
        let mut options = ConnectOptions::new(url.clone());
        options
            .max_connections(8)
            .min_connections(1)
            .set_schema_search_path(TEMPLATE_SCHEMA);
        let timer = Timer::new("pool_connect", &target.schema_name);
        let db = match Database::connect(options).await {
            Ok(db) => Arc::new(db),
            Err(error) => {
                drop_database(&target.database_url, &name).await;
                return Err(error.into());
            }
        };
        drop(timer);
        let lease = Arc::new(TestSchemaLease {
            database: db,
            database_url: url,
            schema_name: TEMPLATE_SCHEMA.to_owned(),
            experiment_database: Some((target.database_url.clone(), name)),
        });
        *current = Arc::downgrade(&lease);
        lease
    };
    Ok(MetadataStore {
        db: Arc::clone(&lease.database),
        postgres_database_url: Some(Arc::from(lease.database_url.clone())),
        _test_schema: Some(lease),
    })
}

pub(super) async fn drop_database(admin_url: &str, name: &str) {
    let Ok(admin) = Database::connect(admin_url).await else {
        return;
    };
    if let Err(error) = execute(
        &admin,
        format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            quote_pg_ident(name)
        ),
    )
    .await
    {
        eprintln!("experiment database cleanup failed for {name}: {error}");
    }
    let _ = admin.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experiment_template_isolates_stores_and_preserves_reopened_target() {
        if !template_enabled().unwrap() {
            return;
        }
        let target = TestDatabaseTarget::required().unwrap();
        let first = connect_postgres_test_store(&target).unwrap();
        let other = connect_postgres_test_store(&TestDatabaseTarget::required().unwrap()).unwrap();
        let db = Arc::clone(&first.db);
        run_test_future(async move {
            db.execute_unprepared("INSERT INTO scope_users (id, handle, email, email_verified) VALUES ('experiment', 'experiment', 'experiment@scope.test', TRUE)").await.unwrap();
        });
        let reopened = connect_postgres_test_store(&target).unwrap();
        assert!(Arc::ptr_eq(&first.db, &reopened.db));
        drop(first);
        run_test_future(async move {
            for (store, expected) in [(reopened, 1_i64), (other, 0_i64)] {
                let count = store
                    .db
                    .query_one(Statement::from_string(
                        store.db.get_database_backend(),
                        "SELECT count(*) AS count FROM scope_users WHERE id = 'experiment'"
                            .to_owned(),
                    ))
                    .await
                    .unwrap()
                    .unwrap()
                    .try_get::<i64>("", "count")
                    .unwrap();
                assert_eq!(count, expected);
                crate::migrations::assert_exact_state(store.db.as_ref())
                    .await
                    .unwrap();
            }
        });
        // Reopening after the final lease drops starts fresh, even during cleanup.
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
}
