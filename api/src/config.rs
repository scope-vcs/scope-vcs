use std::path::{Path as FsPath, PathBuf};

use scope_git::{DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES, GitStorageLimits};

pub(crate) const SCOPE_APP_ORIGIN_ENV: &str = "SCOPE_APP_ORIGIN";
pub(crate) const SCOPE_API_PUBLIC_URL_ENV: &str = "SCOPE_API_PUBLIC_URL";
pub(crate) const SCOPE_GIT_PUBLIC_URL_ENV: &str = "SCOPE_GIT_PUBLIC_URL";
pub(crate) const DATABASE_URL_ENV: &str = "DATABASE_URL";
pub(crate) const SCOPE_DATABASE_MAX_CONNECTIONS_ENV: &str = "SCOPE_DATABASE_MAX_CONNECTIONS";
pub(crate) const SCOPE_REPO_ROOT_ENV: &str = "SCOPE_REPO_ROOT";
pub(crate) const SCOPE_DATA_DIR_ENV: &str = "SCOPE_DATA_DIR";
pub(crate) const SCOPE_OBJECT_ENCRYPTION_KEY_ENV: &str = "SCOPE_OBJECT_ENCRYPTION_KEY";
pub(crate) const SCOPE_OBJECT_STORE_MAX_BYTES_ENV: &str = "SCOPE_OBJECT_STORE_MAX_BYTES";
pub(crate) const SCOPE_GIT_CACHE_MAX_BYTES_ENV: &str = "SCOPE_GIT_CACHE_MAX_BYTES";
pub(crate) const CLERK_ISSUER_ENV: &str = "CLERK_ISSUER";
pub(crate) const CLERK_JWKS_URL_ENV: &str = "CLERK_JWKS_URL";
pub(crate) const CLERK_AUTHORIZED_PARTIES_ENV: &str = "CLERK_AUTHORIZED_PARTIES";
pub(crate) const CLERK_AUDIENCE_ENV: &str = "CLERK_AUDIENCE";
pub(crate) const CLERK_SECRET_KEY_ENV: &str = "CLERK_SECRET_KEY";
pub(crate) const DEFAULT_CLERK_AUDIENCE: &str = "scope-api";
pub(crate) const LOCAL_APP_ORIGIN: &str = "http://localhost:3000";
pub(crate) const LOCAL_API_ORIGIN: &str = "http://localhost:8080";
pub(crate) const FIRST_PUSH_TOKEN_PREFIX: &str = "scope_fp_";
pub(crate) const GIT_PUSH_TOKEN_PREFIX: &str = "scope_git_";
pub(crate) const CLI_SESSION_TOKEN_PREFIX: &str = "scope_cli_";
pub(crate) const RECEIVE_PACK_STAGING_BYTES: usize = 16;
pub(crate) const GIT_UPLOAD_PACK: &str = "git-upload-pack";
pub(crate) const GIT_RECEIVE_PACK: &str = "git-receive-pack";
pub(crate) const DEFAULT_GIT_CACHE_MAX_BYTES: usize = 10 * 1024 * 1024 * 1024;
pub(crate) const AWAITING_FIRST_PUSH_GIT_ERROR: &str = "repo is awaiting its first push";
pub(crate) const MAX_RECEIVE_PACK_BYTES: usize = 512 * 1024 * 1024;
pub(crate) const MAX_UPLOAD_PACK_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_PENDING_IMPORT_BLOB_BYTES: usize = 25 * 1024 * 1024;

pub(crate) fn database_url_from_env() -> anyhow::Result<String> {
    non_empty_env(DATABASE_URL_ENV)
        .ok_or_else(|| anyhow::anyhow!("{DATABASE_URL_ENV} is required for Scope metadata storage"))
}

pub(crate) fn git_public_url_from_env(local_fallback: Option<&str>) -> anyhow::Result<String> {
    git_public_url(std::env::var(SCOPE_GIT_PUBLIC_URL_ENV).ok(), local_fallback)
}

fn git_public_url(
    configured: Option<String>,
    local_fallback: Option<&str>,
) -> anyhow::Result<String> {
    configured
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(local_fallback)
        .map(|value| value.trim_end_matches('/').to_string())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{SCOPE_GIT_PUBLIC_URL_ENV} is required to create repository Git remote metadata"
            )
        })
}

pub(crate) fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

pub(crate) fn git_storage_limits_from_env() -> anyhow::Result<GitStorageLimits> {
    let max_object_bytes = parse_usize_env(
        SCOPE_OBJECT_STORE_MAX_BYTES_ENV,
        DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES,
    )?;
    GitStorageLimits::new(max_object_bytes).map_err(anyhow::Error::from)
}

pub(crate) fn git_cache_max_bytes_from_env() -> anyhow::Result<usize> {
    let bytes = parse_usize_env(SCOPE_GIT_CACHE_MAX_BYTES_ENV, DEFAULT_GIT_CACHE_MAX_BYTES)?;
    if bytes == 0 {
        anyhow::bail!("{SCOPE_GIT_CACHE_MAX_BYTES_ENV} must be greater than zero");
    }
    Ok(bytes)
}

pub(crate) fn database_max_connections_from_env() -> anyhow::Result<u32> {
    let value = parse_positive_usize_env(
        SCOPE_DATABASE_MAX_CONNECTIONS_ENV,
        scope_postgres::db::DEFAULT_METADATA_POOL_MAX_CONNECTIONS as usize,
    )?;
    u32::try_from(value)
        .map_err(|_| anyhow::anyhow!("{SCOPE_DATABASE_MAX_CONNECTIONS_ENV} is too large"))
}

pub(crate) fn parse_positive_usize_env(name: &str, default: usize) -> anyhow::Result<usize> {
    let value = parse_usize_env(name, default)?;
    if value == 0 {
        anyhow::bail!("{name} must be greater than zero");
    }
    Ok(value)
}

pub(crate) fn parse_usize_env(name: &str, default: usize) -> anyhow::Result<usize> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .parse::<usize>()
            .map_err(|error| anyhow::anyhow!("{name} must be an integer: {error}")),
        _ => Ok(default),
    }
}

pub(crate) fn data_dir(repo_root: &FsPath) -> PathBuf {
    non_empty_env(SCOPE_DATA_DIR_ENV)
        .map(|value| {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                repo_root.join(path)
            }
        })
        .unwrap_or_else(|| repo_root.join(".scope"))
}

pub(crate) fn git_repo_root() -> PathBuf {
    if let Some(root) = non_empty_env(SCOPE_REPO_ROOT_ENV) {
        return PathBuf::from(root);
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_git_public_url_is_required() {
        for configured in [None, Some(String::new()), Some(" \t\n".to_string())] {
            let error = git_public_url(configured, None).unwrap_err();
            assert!(error.to_string().contains(SCOPE_GIT_PUBLIC_URL_ENV));
        }
    }

    #[test]
    fn git_public_url_preserves_local_fallback_and_normalizes_configuration() {
        assert_eq!(
            git_public_url(None, Some(LOCAL_API_ORIGIN)).unwrap(),
            LOCAL_API_ORIGIN
        );
        assert_eq!(
            git_public_url(Some(" https://git.scope.example/ ".to_string()), None).unwrap(),
            "https://git.scope.example"
        );
    }
}
