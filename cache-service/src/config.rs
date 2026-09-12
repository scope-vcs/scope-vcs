use scope_object_store::S3ObjectStoreSettings;
use scope_object_store::config::required_env as required;

const DATABASE_URL: &str = "DATABASE_URL";
const CACHE_BACKEND: &str = "SCOPE_CACHE_BACKEND";
const CACHE_GRANT_PUBLIC_KEY: &str = "SCOPE_CACHE_GRANT_PUBLIC_KEY";

pub struct Settings {
    pub(crate) database_url: String,
    pub(crate) object_store: S3ObjectStoreSettings,
    pub(crate) backend: String,
    pub(crate) grant_public_key_pem: String,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        let backend = required(CACHE_BACKEND)?;
        if backend.len() > 64
            || backend.starts_with('-')
            || backend.ends_with('-')
            || backend.contains("--")
            || !backend
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            anyhow::bail!(
                "{CACHE_BACKEND} must contain lowercase letters, digits, or single hyphens"
            );
        }
        let object_store = S3ObjectStoreSettings::from_env("SCOPE_CACHE_BUCKET")?;
        Ok(Self {
            database_url: required(DATABASE_URL)?,
            object_store,
            backend,
            grant_public_key_pem: required(CACHE_GRANT_PUBLIC_KEY)?,
        })
    }
}
