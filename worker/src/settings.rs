use scope_git::{DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES, GitStorageLimits};
use scope_postgres::db::DEFAULT_METADATA_POOL_MAX_CONNECTIONS;
use scope_storage::{
    GitSegmentStoreConfig,
    config::{nonempty_env, required_env},
};
use std::{path::PathBuf, time::Duration};

const DATABASE_URL_ENV: &str = "DATABASE_URL";
const SCOPE_DATA_DIR_ENV: &str = "SCOPE_DATA_DIR";
const SCOPE_OBJECT_STORE_MAX_BYTES_ENV: &str = "SCOPE_OBJECT_STORE_MAX_BYTES";
const SCOPE_GIT_COMPACTION_CONCURRENCY_ENV: &str = "SCOPE_GIT_COMPACTION_CONCURRENCY";
const DEFAULT_HEALTH_PORT: u16 = 8081;

pub(crate) const CONTROL_POLL_BATCH_SIZE: usize = 10;
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(1_000);
pub(crate) const GIT_COMPACTION_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub(crate) struct WorkerSettings {
    pub(crate) database_url: String,
    pub(crate) health_port: u16,
    pub(crate) worker_id: String,
    pub(crate) git_storage_limits: GitStorageLimits,
    pub(crate) git_segment_store: GitSegmentStoreConfig,
    pub(crate) data_dir: PathBuf,
    pub(crate) git_compaction_concurrency: usize,
}

impl WorkerSettings {
    pub(crate) fn metadata_max_connections(&self) -> u32 {
        DEFAULT_METADATA_POOL_MAX_CONNECTIONS
            .saturating_add(u32::try_from(self.git_compaction_concurrency).unwrap_or(u32::MAX))
    }

    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let database_url = required_env(DATABASE_URL_ENV)?;
        let health_port = match nonempty_env("PORT") {
            Some(value) => value
                .parse::<u16>()
                .map_err(|error| anyhow::anyhow!("PORT must be a TCP port: {error}"))?,
            None => DEFAULT_HEALTH_PORT,
        };
        let worker_id = nonempty_env("SCOPE_WORKER_ID").unwrap_or_else(default_worker_id);
        let git_storage_limits = git_storage_limits_from_env()?;
        let data_dir = nonempty_env(SCOPE_DATA_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".scope"));
        let git_segment_store = GitSegmentStoreConfig::from_env(data_dir.join("git-segments"))?;
        let git_compaction_concurrency = parse_usize_env(SCOPE_GIT_COMPACTION_CONCURRENCY_ENV, 1)?;
        if git_compaction_concurrency == 0 {
            anyhow::bail!("{SCOPE_GIT_COMPACTION_CONCURRENCY_ENV} must be at least 1");
        }
        Ok(Self {
            database_url,
            health_port,
            worker_id,
            git_storage_limits,
            git_segment_store,
            data_dir,
            git_compaction_concurrency,
        })
    }
}

fn git_storage_limits_from_env() -> anyhow::Result<GitStorageLimits> {
    GitStorageLimits::new(parse_usize_env(
        SCOPE_OBJECT_STORE_MAX_BYTES_ENV,
        DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES,
    )?)
    .map_err(anyhow::Error::from)
}

fn parse_usize_env(name: &str, default: usize) -> anyhow::Result<usize> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .parse::<usize>()
            .map_err(|error| anyhow::anyhow!("{name} must be an integer: {error}")),
        _ => Ok(default),
    }
}

fn default_worker_id() -> String {
    let host = std::env::var("RAILWAY_REPLICA_ID")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "local".to_string());
    format!("scope-worker-{host}-{}", std::process::id())
}
