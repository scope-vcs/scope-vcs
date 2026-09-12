use crate::config::{
    SCOPE_BUCKET_FORCE_PATH_STYLE_ENV, SCOPE_OBJECT_ENCRYPTION_KEY_ENV, non_empty_env,
};
#[cfg(feature = "local-dev")]
use scope_git_storage::FileMultipartStore;
use scope_git_storage::{
    GitSegmentStore, S3MultipartSettings, S3MultipartStore, SegmentEncryptionKey,
};
#[cfg(feature = "local-dev")]
use scope_object_store::{FileObjectStore, FileObjectStoreSettings};
use scope_object_store::{S3ObjectStore, S3ObjectStoreSettings};
#[cfg(feature = "local-dev")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "local-dev")]
const SCOPE_OBJECT_STORE_DIR_ENV: &str = "SCOPE_OBJECT_STORE_DIR";

pub(crate) fn encryption_key_from_env() -> anyhow::Result<[u8; 32]> {
    scope_object_store::config::encryption_key_from_env(SCOPE_OBJECT_ENCRYPTION_KEY_ENV)
        .map_err(anyhow::Error::from)
}

pub(crate) fn s3_from_env() -> anyhow::Result<S3ObjectStore> {
    S3ObjectStore::new(s3_settings_from_env()?).map_err(anyhow::Error::from)
}

pub(crate) fn s3_settings_from_env() -> anyhow::Result<S3ObjectStoreSettings> {
    let mut settings = S3ObjectStoreSettings::from_env("SCOPE_BUCKET")?;
    settings.force_path_style = non_empty_env(SCOPE_BUCKET_FORCE_PATH_STYLE_ENV)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    Ok(settings)
}

pub(crate) fn git_segment_store_from_env(
    local_root: PathBuf,
    encryption_key: [u8; 32],
) -> anyhow::Result<GitSegmentStore> {
    let s3 = s3_settings_from_env()?;
    let backend = S3MultipartStore::new(S3MultipartSettings {
        endpoint: s3.endpoint,
        bucket: s3.bucket,
        region: s3.region,
        access_key_id: s3.access_key_id,
        secret_access_key: s3.secret_access_key,
        force_path_style: s3.force_path_style,
    })?;
    let key = SegmentEncryptionKey::new("primary", encryption_key)?;
    GitSegmentStore::new(
        std::sync::Arc::new(backend),
        key,
        crate::config::git_segment_store_config_from_env(local_root)?,
    )
    .map_err(anyhow::Error::from)
}

#[cfg(feature = "local-dev")]
pub(crate) fn file_from_env(default_root: &Path) -> FileObjectStore {
    FileObjectStore::new(FileObjectStoreSettings::new(filesystem_root_from_env(
        default_root,
    )))
}

#[cfg(feature = "local-dev")]
pub(crate) fn git_segment_file_store_from_env(
    default_root: &Path,
) -> anyhow::Result<FileMultipartStore> {
    FileMultipartStore::new(filesystem_root_from_env(default_root)).map_err(anyhow::Error::from)
}

#[cfg(feature = "local-dev")]
fn filesystem_root_from_env(default_root: &Path) -> PathBuf {
    non_empty_env(SCOPE_OBJECT_STORE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| default_root.to_path_buf())
}
