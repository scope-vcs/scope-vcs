use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_object_store::{FileObjectStoreSettings, S3ObjectStoreSettings};
use std::path::PathBuf;

const DATABASE_URL: &str = "DATABASE_URL";
const MEDIA_BUCKET_ENDPOINT: &str = "SCOPE_MEDIA_BUCKET_ENDPOINT";
const MEDIA_BUCKET_NAME: &str = "SCOPE_MEDIA_BUCKET_NAME";
const MEDIA_BUCKET_REGION: &str = "SCOPE_MEDIA_BUCKET_REGION";
const MEDIA_BUCKET_ACCESS_KEY_ID: &str = "SCOPE_MEDIA_BUCKET_ACCESS_KEY_ID";
const MEDIA_BUCKET_SECRET_ACCESS_KEY: &str = "SCOPE_MEDIA_BUCKET_SECRET_ACCESS_KEY";
const MEDIA_BUCKET_FORCE_PATH_STYLE: &str = "SCOPE_MEDIA_BUCKET_FORCE_PATH_STYLE";
const MEDIA_ENCRYPTION_KEY: &str = "SCOPE_MEDIA_ENCRYPTION_KEY";
const MEDIA_GRANT_PUBLIC_KEY: &str = "SCOPE_MEDIA_GRANT_PUBLIC_KEY";
const MEDIA_OBJECT_STORE: &str = "SCOPE_MEDIA_OBJECT_STORE";
const MEDIA_OBJECT_STORE_DIR: &str = "SCOPE_MEDIA_OBJECT_STORE_DIR";
const MEDIA_BLOCKING_OPERATIONS: &str = "SCOPE_MEDIA_MAX_BLOCKING_OPERATIONS";
const MEDIA_ALLOWED_ORIGIN: &str = "SCOPE_MEDIA_ALLOWED_ORIGIN";
const MEDIA_CONCURRENT_UPLOADS: &str = "SCOPE_MEDIA_MAX_CONCURRENT_UPLOADS";
const MEDIA_CONCURRENT_READS: &str = "SCOPE_MEDIA_MAX_CONCURRENT_READS";

const DEFAULT_BLOCKING_OPERATIONS: usize = 4;
const DEFAULT_CONCURRENT_UPLOADS: usize = 4;
const DEFAULT_CONCURRENT_READS: usize = 8;

pub(crate) enum MediaObjectStoreSettings {
    Filesystem(FileObjectStoreSettings),
    S3(S3ObjectStoreSettings),
}

pub struct Settings {
    pub(crate) database_url: String,
    pub(crate) object_store: MediaObjectStoreSettings,
    pub(crate) encryption_key: [u8; 32],
    pub(crate) grant_public_key_pem: String,
    pub(crate) max_blocking_operations: usize,
    pub(crate) max_concurrent_uploads: usize,
    pub(crate) max_concurrent_reads: usize,
    pub(crate) allowed_origin: Option<String>,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        let object_store = match optional(MEDIA_OBJECT_STORE).as_deref() {
            Some("filesystem") => {
                MediaObjectStoreSettings::Filesystem(FileObjectStoreSettings::new(
                    optional(MEDIA_OBJECT_STORE_DIR)
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from("data/media-objects")),
                ))
            }
            Some(value) if value != "s3" => {
                anyhow::bail!("unsupported {MEDIA_OBJECT_STORE} value {value}")
            }
            _ => MediaObjectStoreSettings::S3(s3_settings_from_env()?),
        };
        let max_blocking_operations = optional(MEDIA_BLOCKING_OPERATIONS)
            .map(|value| value.parse::<usize>())
            .transpose()?
            .unwrap_or(DEFAULT_BLOCKING_OPERATIONS);
        if max_blocking_operations == 0 || max_blocking_operations > 64 {
            anyhow::bail!("{MEDIA_BLOCKING_OPERATIONS} must be between 1 and 64")
        }
        let allowed_origin = optional(MEDIA_ALLOWED_ORIGIN)
            .map(validate_origin)
            .transpose()?;
        let max_concurrent_uploads =
            positive_limit(MEDIA_CONCURRENT_UPLOADS, DEFAULT_CONCURRENT_UPLOADS)?;
        let max_concurrent_reads =
            positive_limit(MEDIA_CONCURRENT_READS, DEFAULT_CONCURRENT_READS)?;
        Ok(Self {
            database_url: required(DATABASE_URL)?,
            object_store,
            encryption_key: encryption_key(&required(MEDIA_ENCRYPTION_KEY)?)?,
            grant_public_key_pem: required(MEDIA_GRANT_PUBLIC_KEY)?,
            max_blocking_operations,
            max_concurrent_uploads,
            max_concurrent_reads,
            allowed_origin,
        })
    }

    pub fn allowed_origin(&self) -> Option<&str> {
        self.allowed_origin.as_deref()
    }
}

fn s3_settings_from_env() -> anyhow::Result<S3ObjectStoreSettings> {
    let endpoint = required(MEDIA_BUCKET_ENDPOINT)?;
    validate_origin_url(MEDIA_BUCKET_ENDPOINT, &endpoint)?;
    let mut settings = S3ObjectStoreSettings::new(
        endpoint,
        required(MEDIA_BUCKET_NAME)?,
        required(MEDIA_BUCKET_REGION)?,
        required(MEDIA_BUCKET_ACCESS_KEY_ID)?,
        required(MEDIA_BUCKET_SECRET_ACCESS_KEY)?,
    );
    settings.force_path_style = optional(MEDIA_BUCKET_FORCE_PATH_STYLE)
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes"));
    Ok(settings)
}

fn encryption_key(encoded: &str) -> anyhow::Result<[u8; 32]> {
    let decoded = BASE64
        .decode(encoded)
        .map_err(|_| anyhow::anyhow!("{MEDIA_ENCRYPTION_KEY} must be valid base64"))?;
    decoded.try_into().map_err(|bytes: Vec<u8>| {
        anyhow::anyhow!(
            "{MEDIA_ENCRYPTION_KEY} must decode to 32 bytes, got {}",
            bytes.len()
        )
    })
}

fn validate_origin(origin: String) -> anyhow::Result<String> {
    validate_origin_url(MEDIA_ALLOWED_ORIGIN, &origin)?;
    Ok(origin)
}

fn validate_origin_url(name: &str, value: &str) -> anyhow::Result<()> {
    let parsed = url::Url::parse(value).map_err(|_| anyhow::anyhow!("{name} is not a URL"))?;
    let local = matches!(
        parsed.host(),
        Some(url::Host::Domain("localhost"))
            | Some(url::Host::Ipv4(std::net::Ipv4Addr::LOCALHOST))
            | Some(url::Host::Ipv6(std::net::Ipv6Addr::LOCALHOST))
    );
    let secure = parsed.scheme() == "https" || (parsed.scheme() == "http" && local);
    if !secure
        || parsed.cannot_be_a_base()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.origin().ascii_serialization() != value
    {
        anyhow::bail!("{name} must be an exact HTTPS origin, or an HTTP loopback origin")
    }
    Ok(())
}

fn positive_limit(name: &str, default: usize) -> anyhow::Result<usize> {
    let value = optional(name)
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(default);
    if value == 0 || value > 64 {
        anyhow::bail!("{name} must be between 1 and 64")
    }
    Ok(value)
}

fn required(name: &str) -> anyhow::Result<String> {
    optional(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

fn optional(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_key_must_be_exactly_32_bytes() {
        let key = BASE64.encode([7_u8; 32]);
        assert_eq!(encryption_key(&key).unwrap(), [7_u8; 32]);
        assert!(encryption_key(&BASE64.encode([7_u8; 31])).is_err());
        assert!(encryption_key("not base64").is_err());
    }

    #[test]
    fn browser_origin_has_no_trailing_slash_or_insecure_remote_scheme() {
        assert_eq!(
            validate_origin("https://scopevcs.com".to_string()).unwrap(),
            "https://scopevcs.com"
        );
        assert!(validate_origin("https://scopevcs.com/".to_string()).is_err());
        assert!(validate_origin("http://scopevcs.com".to_string()).is_err());
        assert!(validate_origin("http://localhost:3000".to_string()).is_ok());
        assert!(validate_origin("http://127.0.0.1:3000".to_string()).is_ok());
        assert!(validate_origin("http://[::1]:3000".to_string()).is_ok());
        assert!(validate_origin("http://localhost.attacker:3000".to_string()).is_err());
        assert!(validate_origin("https://scopevcs.com/path".to_string()).is_err());
    }
}
