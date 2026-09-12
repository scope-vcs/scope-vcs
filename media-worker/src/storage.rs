use scope_media_storage::MediaStorage;
use scope_object_store::config::nonempty_env;
use scope_object_store::{
    FileObjectStore, FileObjectStoreSettings, ObjectStore, S3ObjectStore, S3ObjectStoreSettings,
};
use std::sync::Arc;

const ENCRYPTION_KEY_ENV: &str = "SCOPE_MEDIA_ENCRYPTION_KEY";
const STORE_ENV: &str = "SCOPE_MEDIA_OBJECT_STORE";
const STORE_DIR_ENV: &str = "SCOPE_MEDIA_OBJECT_STORE_DIR";

pub fn from_env() -> anyhow::Result<MediaStorage> {
    let raw: Arc<dyn ObjectStore> = match nonempty_env(STORE_ENV).as_deref() {
        Some("filesystem") => {
            let root = nonempty_env(STORE_DIR_ENV)
                .map(Into::into)
                .unwrap_or_else(|| "/tmp/scope-media-objects".into());
            Arc::new(FileObjectStore::new(FileObjectStoreSettings::new(root)))
        }
        Some(value) if value != "s3" => anyhow::bail!("unsupported {STORE_ENV} value {value}"),
        _ => Arc::new(S3ObjectStore::new(s3_settings()?)?),
    };
    MediaStorage::encrypted(raw, encryption_key()?, 2).map_err(anyhow::Error::from)
}

fn s3_settings() -> anyhow::Result<S3ObjectStoreSettings> {
    Ok(S3ObjectStoreSettings::from_env("SCOPE_MEDIA_BUCKET")?)
}

fn encryption_key() -> anyhow::Result<[u8; 32]> {
    Ok(scope_object_store::config::encryption_key_from_env(
        ENCRYPTION_KEY_ENV,
    )?)
}
