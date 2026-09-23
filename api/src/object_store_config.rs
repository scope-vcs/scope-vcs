use crate::config::SCOPE_OBJECT_ENCRYPTION_KEY_ENV;
#[cfg(feature = "local-dev")]
use crate::config::non_empty_env;
#[cfg(feature = "local-dev")]
use scope_storage::FileBackend;
use scope_storage::{ObjectBackend, S3Backend, S3Settings};
#[cfg(feature = "local-dev")]
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(feature = "local-dev")]
const SCOPE_OBJECT_STORE_DIR_ENV: &str = "SCOPE_OBJECT_STORE_DIR";

pub(crate) fn encryption_key_from_env() -> anyhow::Result<[u8; 32]> {
    scope_storage::config::encryption_key_from_env(SCOPE_OBJECT_ENCRYPTION_KEY_ENV)
        .map_err(anyhow::Error::from)
}

pub(crate) fn s3_settings_from_env() -> anyhow::Result<S3Settings> {
    S3Settings::from_env("SCOPE_BUCKET").map_err(anyhow::Error::from)
}

pub(crate) fn s3_backend_from_env() -> anyhow::Result<Arc<dyn ObjectBackend>> {
    Ok(Arc::new(S3Backend::new(s3_settings_from_env()?)?))
}

#[cfg(feature = "local-dev")]
pub(crate) fn file_backend_from_env(default_root: &Path) -> anyhow::Result<Arc<dyn ObjectBackend>> {
    let root = non_empty_env(SCOPE_OBJECT_STORE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| default_root.to_path_buf());
    Ok(Arc::new(FileBackend::new(root)?))
}
