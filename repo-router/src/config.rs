use anyhow::Context;
use std::time::Duration;

const BACKEND_ENV: &str = "SCOPE_REPO_ROUTER_BACKEND";
const READ_REPLICAS_ENV: &str = "SCOPE_REPO_ROUTER_READ_REPLICAS";

const DEFAULT_DNS_REFRESH_MILLIS: u64 = 1_000;
const DEFAULT_DNS_MAX_STALE_MILLIS: u64 = 30_000;
const DEFAULT_CONNECT_TIMEOUT_MILLIS: u64 = 2_000;
const DEFAULT_READ_TIMEOUT_MILLIS: u64 = 60_000;
const DEFAULT_READ_REPLICAS: usize = 1;
const DEFAULT_UPLOAD_PACK_REPLAY_MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct RouterConfig {
    pub backend_authority: String,
    pub dns_refresh: Duration,
    pub dns_max_stale: Duration,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub read_replicas: usize,
    pub upload_pack_replay_max_bytes: usize,
}

impl RouterConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let backend_authority = std::env::var(BACKEND_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("{BACKEND_ENV} is required"))?;
        validate_authority(&backend_authority)?;
        Ok(Self {
            backend_authority,
            dns_refresh: Duration::from_millis(DEFAULT_DNS_REFRESH_MILLIS),
            dns_max_stale: Duration::from_millis(DEFAULT_DNS_MAX_STALE_MILLIS),
            connect_timeout: Duration::from_millis(DEFAULT_CONNECT_TIMEOUT_MILLIS),
            read_timeout: Duration::from_millis(DEFAULT_READ_TIMEOUT_MILLIS),
            read_replicas: positive_usize_from_env(READ_REPLICAS_ENV, DEFAULT_READ_REPLICAS)?,
            upload_pack_replay_max_bytes: DEFAULT_UPLOAD_PACK_REPLAY_MAX_BYTES,
        })
    }
}

fn positive_usize_from_env(name: &str, default: usize) -> anyhow::Result<usize> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => parse_positive_usize(name, &value),
        _ => Ok(default),
    }
}

fn parse_positive_usize(name: &str, value: &str) -> anyhow::Result<usize> {
    let value = value
        .parse::<usize>()
        .with_context(|| format!("{name} must be an integer"))?;
    if value == 0 {
        anyhow::bail!("{name} must be greater than zero");
    }
    Ok(value)
}

fn validate_authority(authority: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(&format!("http://{authority}"))
        .with_context(|| format!("{BACKEND_ENV} must be a host and port"))?;
    if url.host_str().is_none() || url.port().is_none() || url.path() != "/" {
        anyhow::bail!("{BACKEND_ENV} must be a host and port");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_authority_requires_an_explicit_port() {
        assert!(validate_authority("scope-api.railway.internal:8080").is_ok());
        assert!(validate_authority("scope-api.railway.internal").is_err());
        assert!(validate_authority("https://scope-api.invalid:8080").is_err());
        assert!(validate_authority("scope-api.invalid:8080/path").is_err());
    }

    #[test]
    fn read_replica_count_must_be_positive() {
        assert_eq!(parse_positive_usize(READ_REPLICAS_ENV, "3").unwrap(), 3);
        assert!(parse_positive_usize(READ_REPLICAS_ENV, "0").is_err());
        assert!(parse_positive_usize(READ_REPLICAS_ENV, "many").is_err());
    }
}
