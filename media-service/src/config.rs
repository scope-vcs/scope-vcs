use scope_media_storage::MediaStorageSettings;
use scope_service_runtime::http::ServiceEndpoint;
use scope_storage::config::required_env as required;

const DATABASE_URL: &str = "DATABASE_URL";
const MEDIA_GRANT_PUBLIC_KEY: &str = "SCOPE_MEDIA_GRANT_PUBLIC_KEY";
const MEDIA_ALLOWED_ORIGIN: &str = "SCOPE_MEDIA_ALLOWED_ORIGIN";

const DEFAULT_STORAGE_OPERATIONS: usize = 4;
const DEFAULT_CONCURRENT_UPLOADS: usize = 4;
const DEFAULT_CONCURRENT_READS: usize = 8;

pub struct Settings {
    pub(crate) database_url: String,
    pub(crate) storage: MediaStorageSettings,
    pub(crate) grant_public_key_pem: String,
    pub(crate) max_storage_operations: usize,
    pub(crate) max_concurrent_uploads: usize,
    pub(crate) max_concurrent_reads: usize,
    pub(crate) allowed_origin: String,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        let storage = MediaStorageSettings::from_env()?;
        let allowed_origin = validate_origin(required(MEDIA_ALLOWED_ORIGIN)?)?;
        Ok(Self {
            database_url: required(DATABASE_URL)?,
            storage,
            grant_public_key_pem: required(MEDIA_GRANT_PUBLIC_KEY)?,
            max_storage_operations: DEFAULT_STORAGE_OPERATIONS,
            max_concurrent_uploads: DEFAULT_CONCURRENT_UPLOADS,
            max_concurrent_reads: DEFAULT_CONCURRENT_READS,
            allowed_origin,
        })
    }

    pub fn allowed_origin(&self) -> &str {
        &self.allowed_origin
    }
}

const HTTP_LOOPBACK_ORIGINS: [&str; 3] = ["http://localhost", "http://127.0.0.1", "http://[::1]"];

fn validate_origin(origin: String) -> anyhow::Result<String> {
    let exact =
        ServiceEndpoint::parse_origin(&origin).is_ok_and(|endpoint| endpoint.as_str() == origin);
    let strict_loopback = !origin.starts_with("http:")
        || HTTP_LOOPBACK_ORIGINS.iter().any(|loopback| {
            origin
                .strip_prefix(loopback)
                .is_some_and(|port| port.is_empty() || port.starts_with(':'))
        });
    if !(exact && strict_loopback) {
        anyhow::bail!(
            "{MEDIA_ALLOWED_ORIGIN} must be an exact HTTPS origin, or an HTTP loopback origin"
        )
    }
    Ok(origin)
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
        assert!(validate_origin("http://127.0.0.2:3000".to_string()).is_err());
        assert!(validate_origin("https://scopevcs.com:443".to_string()).is_err());
        assert!(validate_origin("https://scopevcs.com/path".to_string()).is_err());
    }
}
