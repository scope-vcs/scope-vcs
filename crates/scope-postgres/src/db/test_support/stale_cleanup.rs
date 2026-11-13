use super::*;
use std::time::Duration;

const ABANDONED_AFTER: Duration = Duration::from_secs(60 * 60);
pub(super) const SWEEP_BUDGET: Duration = Duration::from_secs(10);

const DATABASE_NAMES: &str =
    "SELECT datname FROM pg_database WHERE datname LIKE 'scope\\_test\\_%'";
const SCHEMA_NAMES: &str = "SELECT nspname FROM pg_namespace WHERE nspname LIKE 'scope\\_test\\_%'";

pub(super) async fn sweep(admin_url: &str, deadline: tokio::time::Instant) -> anyhow::Result<()> {
    let admin = Database::connect(admin_url).await?;
    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let databases = abandoned(&admin, DATABASE_NAMES, now_nanos).await?;
    let schemas = abandoned(&admin, SCHEMA_NAMES, now_nanos).await?;
    let drops = schemas
        .iter()
        .map(|schema| format!("DROP SCHEMA IF EXISTS {} CASCADE", quote_pg_ident(schema)))
        .chain(databases.iter().map(|database| {
            format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                quote_pg_ident(database)
            )
        }));
    for drop in drops {
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        if let Err(error) = admin.execute_unprepared(&drop).await {
            eprintln!("sweeping abandoned PostgreSQL test state failed at `{drop}`: {error}");
        }
    }
    admin.close().await?;
    Ok(())
}

#[cfg(unix)]
pub(super) fn sweep_process_servers(
    parent: &std::path::Path,
    bin: &std::path::Path,
) -> anyhow::Result<()> {
    use super::process_os::{effective_user_id, process_is_alive};
    use std::{fs, os::unix::fs::MetadataExt as _};

    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(pid) = server_owner_pid(name) else {
            continue;
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_dir() || metadata.uid() != effective_user_id() || process_is_alive(pid) {
            continue;
        }
        let directory = match fs::File::open(entry.path()) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        match directory.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        if entry.path().exists() && !process_is_alive(pid) {
            super::process_server::stop(&entry.path(), bin)?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn sweep_process_servers(
    _: &std::path::Path,
    _: &std::path::Path,
) -> anyhow::Result<()> {
    anyhow::bail!("PostgreSQL test servers require a Unix host")
}

#[cfg(unix)]
fn server_owner_pid(name: &str) -> Option<libc::pid_t> {
    let (pid, suffix) = name.strip_prefix("scope-pg-")?.split_once('-')?;
    if suffix.len() != 16 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    pid.parse::<libc::pid_t>().ok().filter(|pid| *pid > 0)
}

async fn abandoned(
    admin: &DatabaseConnection,
    names_sql: &str,
    now_nanos: u128,
) -> anyhow::Result<Vec<String>> {
    let rows = admin
        .query_all_raw(Statement::from_string(
            admin.get_database_backend(),
            names_sql.to_owned(),
        ))
        .await?;
    let mut names = Vec::new();
    for row in rows {
        let name: String = row.try_get_by_index(0)?;
        if is_abandoned(&name, now_nanos) {
            names.push(name);
        }
    }
    Ok(names)
}

fn is_abandoned(name: &str, now_nanos: u128) -> bool {
    let Some(rest) = name.strip_prefix("scope_test_") else {
        return false;
    };
    let rest = rest
        .strip_prefix("db_")
        .or_else(|| rest.strip_prefix("template_"))
        .unwrap_or(rest);
    let parts = rest.split('_').collect::<Vec<_>>();
    let [process_id, created_nanos, sequence] = parts.as_slice() else {
        return false;
    };
    if process_id.parse::<u32>().is_err() || sequence.parse::<u64>().is_err() {
        return false;
    }
    created_nanos
        .parse::<u128>()
        .is_ok_and(|created| now_nanos.saturating_sub(created) > ABANDONED_AFTER.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u128 = 1_790_000_000_000_000_000;
    const TWO_HOURS: u128 = 2 * 60 * 60 * 1_000_000_000;
    const TEN_MINUTES: u128 = 10 * 60 * 1_000_000_000;

    #[test]
    fn only_old_harness_names_are_abandoned() {
        for name in [
            format!("scope_test_db_4242_{}_7", NOW - TWO_HOURS),
            format!("scope_test_template_4242_{}_0", NOW - TWO_HOURS),
            format!("scope_test_4242_{}_19", NOW - TWO_HOURS),
        ] {
            assert!(is_abandoned(&name, NOW), "{name}");
        }
        for name in [
            format!("scope_test_db_4242_{}_7", NOW - TEN_MINUTES),
            format!("scope_test_4242_{}_19", NOW - TEN_MINUTES),
            "scope_test".to_string(),
            "scope_test_template".to_string(),
            "scope_test_run".to_string(),
            format!("scope_test_archive_4242_{}_7", NOW - TWO_HOURS),
            format!("scope_test_db_{}_7", NOW - TWO_HOURS),
            format!("scope_dev_4242_{}_7", NOW - TWO_HOURS),
        ] {
            assert!(!is_abandoned(&name, NOW), "{name}");
        }
    }
}
