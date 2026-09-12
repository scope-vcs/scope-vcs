use scope_object_store::config::{nonempty_env as optional, required_env as required};
use scope_object_store::{FileObjectStoreSettings, S3ObjectStoreSettings};
use std::path::PathBuf;

const DATABASE_URL: &str = "DATABASE_URL";
const MEDIA_ENCRYPTION_KEY: &str = "SCOPE_MEDIA_ENCRYPTION_KEY";
const MEDIA_GRANT_PUBLIC_KEY: &str = "SCOPE_MEDIA_GRANT_PUBLIC_KEY";
const MEDIA_OBJECT_STORE: &str = "SCOPE_MEDIA_OBJECT_STORE";
const MEDIA_OBJECT_STORE_DIR: &str = "SCOPE_MEDIA_OBJECT_STORE_DIR";
const MEDIA_ALLOWED_ORIGIN: &str = "SCOPE_MEDIA_ALLOWED_ORIGIN";

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
    pub(crate) allowed_origin: String,
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
        let allowed_origin = validate_origin(required(MEDIA_ALLOWED_ORIGIN)?)?;
        Ok(Self {
            database_url: required(DATABASE_URL)?,
            object_store,
            encryption_key: scope_object_store::config::encryption_key_from_env(
                MEDIA_ENCRYPTION_KEY,
            )?,
            grant_public_key_pem: required(MEDIA_GRANT_PUBLIC_KEY)?,
            max_blocking_operations: DEFAULT_BLOCKING_OPERATIONS,
            max_concurrent_uploads: DEFAULT_CONCURRENT_UPLOADS,
            max_concurrent_reads: DEFAULT_CONCURRENT_READS,
            allowed_origin,
        })
    }

    pub fn allowed_origin(&self) -> &str {
        &self.allowed_origin
    }
}

fn s3_settings_from_env() -> anyhow::Result<S3ObjectStoreSettings> {
    Ok(S3ObjectStoreSettings::from_env("SCOPE_MEDIA_BUCKET")?)
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

#[cfg(test)]
mod tests {
    use super::*;

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
