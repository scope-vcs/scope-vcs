use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_media_storage::MediaStorage;
use scope_object_store::{
    FileObjectStore, FileObjectStoreSettings, ObjectStore, S3ObjectStore, S3ObjectStoreSettings,
};
use std::sync::Arc;

const ENDPOINT_ENV: &str = "SCOPE_MEDIA_BUCKET_ENDPOINT";
const BUCKET_ENV: &str = "SCOPE_MEDIA_BUCKET_NAME";
const REGION_ENV: &str = "SCOPE_MEDIA_BUCKET_REGION";
const ACCESS_KEY_ENV: &str = "SCOPE_MEDIA_BUCKET_ACCESS_KEY_ID";
const SECRET_KEY_ENV: &str = "SCOPE_MEDIA_BUCKET_SECRET_ACCESS_KEY";
const FORCE_PATH_STYLE_ENV: &str = "SCOPE_MEDIA_BUCKET_FORCE_PATH_STYLE";
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
    let mut settings = S3ObjectStoreSettings::new(
        required_env(ENDPOINT_ENV)?,
        required_env(BUCKET_ENV)?,
        required_env(REGION_ENV)?,
        required_env(ACCESS_KEY_ENV)?,
        required_env(SECRET_KEY_ENV)?,
    );
    settings.force_path_style = nonempty_env(FORCE_PATH_STYLE_ENV)
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"));
    Ok(settings)
}

fn encryption_key() -> anyhow::Result<[u8; 32]> {
    let encoded = required_env(ENCRYPTION_KEY_ENV)?;
    let decoded = BASE64
        .decode(encoded.trim())
        .map_err(|error| anyhow::anyhow!("{ENCRYPTION_KEY_ENV} must be base64: {error}"))?;
    decoded
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("{ENCRYPTION_KEY_ENV} must decode to exactly 32 bytes"))
}

fn required_env(name: &str) -> anyhow::Result<String> {
    nonempty_env(name).ok_or_else(|| anyhow::anyhow!("{name} must be set"))
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}
