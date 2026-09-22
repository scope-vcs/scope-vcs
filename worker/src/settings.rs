use scope_git::{DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES, GitStorageLimits};
use scope_git_storage::GitSegmentStoreConfig;
use std::{path::PathBuf, time::Duration};

const DATABASE_URL_ENV: &str = "DATABASE_URL";
const SCOPE_DATA_DIR_ENV: &str = "SCOPE_DATA_DIR";
const SCOPE_OBJECT_STORE_MAX_BYTES_ENV: &str = "SCOPE_OBJECT_STORE_MAX_BYTES";
const SCOPE_GIT_SEGMENT_CHUNK_BYTES_ENV: &str = "SCOPE_GIT_SEGMENT_CHUNK_BYTES";
const SCOPE_GIT_SEGMENT_MULTIPART_PART_BYTES_ENV: &str = "SCOPE_GIT_SEGMENT_MULTIPART_PART_BYTES";
const SCOPE_GIT_SEGMENT_CHANNEL_CAPACITY_ENV: &str = "SCOPE_GIT_SEGMENT_CHANNEL_CAPACITY";
const CLOUD_RUN_MAX_CONCURRENCY_ENV: &str = "SCOPE_CLOUD_RUN_MAX_CONCURRENCY";
const DEFAULT_HEALTH_PORT: u16 = 8081;

/// Outbox jobs claimed per control poll.
pub(crate) const BATCH_SIZE: usize = 10;
/// Idle wait between polls for every worker loop.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(1_000);
/// Bound on one compaction's external git work.
pub(crate) const GIT_COMPACTION_TIMEOUT: Duration = Duration::from_secs(120);
/// Cloud run attempts one worker admits concurrently.
const CLOUD_RUN_MAX_CONCURRENCY: usize = 20;

#[derive(Clone)]
pub(crate) struct WorkerSettings {
    pub(crate) database_url: String,
    pub(crate) health_port: u16,
    pub(crate) worker_id: String,
    pub(crate) git_storage_limits: GitStorageLimits,
    pub(crate) git_segment_store: GitSegmentStoreConfig,
    pub(crate) data_dir: PathBuf,
    pub(crate) execution: Option<CloudExecutionSettings>,
}

#[derive(Clone)]
pub(crate) struct CloudExecutionSettings {
    pub(crate) aws_region: String,
    pub(crate) dispatch_broker_function_arn: String,
    pub(crate) runtime_version: String,
    pub(crate) max_concurrency: usize,
}

impl WorkerSettings {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let database_url = required_env(DATABASE_URL_ENV)?;
        let health_port = match non_empty_env("PORT") {
            Some(value) => value
                .parse::<u16>()
                .map_err(|error| anyhow::anyhow!("PORT must be a TCP port: {error}"))?,
            None => DEFAULT_HEALTH_PORT,
        };
        let worker_id = std::env::var("SCOPE_WORKER_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(default_worker_id);
        let git_storage_limits = git_storage_limits_from_env()?;
        let data_dir = non_empty_env(SCOPE_DATA_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".scope"));
        let mut git_segment_store = GitSegmentStoreConfig::new(data_dir.join("git-segments"));
        git_segment_store.chunk_bytes = parse_usize_env(
            SCOPE_GIT_SEGMENT_CHUNK_BYTES_ENV,
            git_segment_store.chunk_bytes,
        )?;
        git_segment_store.multipart_part_bytes = parse_usize_env(
            SCOPE_GIT_SEGMENT_MULTIPART_PART_BYTES_ENV,
            git_segment_store.multipart_part_bytes,
        )?;
        git_segment_store.channel_capacity = parse_usize_env(
            SCOPE_GIT_SEGMENT_CHANNEL_CAPACITY_ENV,
            git_segment_store.channel_capacity,
        )?;
        Ok(Self {
            database_url,
            health_port,
            worker_id,
            git_storage_limits,
            git_segment_store,
            data_dir,
            execution: cloud_execution_from_env()?,
        })
    }
}

fn cloud_execution_from_env() -> anyhow::Result<Option<CloudExecutionSettings>> {
    let enabled = non_empty_env("SCOPE_CLOUD_RUNS_ENABLED")
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
    if !enabled {
        return Ok(None);
    }
    let aws_region = required_env("AWS_REGION")?;
    let dispatch_broker_function_arn = parse_broker_function_arn(
        &required_env("SCOPE_DISPATCH_BROKER_FUNCTION_ARN")?,
        &aws_region,
    )?;
    Ok(Some(CloudExecutionSettings {
        aws_region,
        dispatch_broker_function_arn,
        runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        max_concurrency: parse_usize_env(CLOUD_RUN_MAX_CONCURRENCY_ENV, CLOUD_RUN_MAX_CONCURRENCY)?,
    }))
}

fn parse_broker_function_arn(value: &str, aws_region: &str) -> anyhow::Result<String> {
    let fields = value.split(':').collect::<Vec<_>>();
    if !(fields.len() == 7 || fields.len() == 8)
        || fields[0] != "arn"
        || !matches!(fields[1], "aws" | "aws-us-gov" | "aws-cn")
        || fields[2] != "lambda"
        || fields[3] != aws_region
        || fields[4].len() != 12
        || !fields[4].bytes().all(|byte| byte.is_ascii_digit())
        || fields[5] != "function"
        || fields[6..].iter().any(|field| {
            field.is_empty()
                || !field
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    {
        anyhow::bail!(
            "SCOPE_DISPATCH_BROKER_FUNCTION_ARN must be an exact Lambda function ARN in AWS_REGION"
        );
    }
    Ok(value.to_owned())
}

pub(crate) fn required_env(name: &str) -> anyhow::Result<String> {
    non_empty_env(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

pub(crate) fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_function_must_be_exact_and_region_bound() {
        let arn = "arn:aws:lambda:us-east-1:123456789012:function:scope-dispatch";
        assert_eq!(parse_broker_function_arn(arn, "us-east-1").unwrap(), arn);
        assert!(parse_broker_function_arn(&format!("{arn}:live"), "us-east-1").is_ok());
        for invalid in [
            "scope-dispatch",
            "arn:aws:lambda:us-east-1:123456789012:function:*",
            "arn:aws:lambda:us-east-1:123456789012:function:",
        ] {
            assert!(parse_broker_function_arn(invalid, "us-east-1").is_err());
        }
        assert!(parse_broker_function_arn(arn, "us-west-2").is_err());
    }
}
