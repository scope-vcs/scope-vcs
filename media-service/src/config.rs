use scope_media_storage::MediaStorageSettings;

const DATABASE_URL: &str = "DATABASE_URL";
const MEDIA_GRANT_PUBLIC_KEY: &str = "SCOPE_MEDIA_GRANT_PUBLIC_KEY";
const MEDIA_BLOCKING_OPERATIONS: &str = "SCOPE_MEDIA_MAX_BLOCKING_OPERATIONS";
const MEDIA_ALLOWED_ORIGIN: &str = "SCOPE_MEDIA_ALLOWED_ORIGIN";
const MEDIA_CONCURRENT_UPLOADS: &str = "SCOPE_MEDIA_MAX_CONCURRENT_UPLOADS";
const MEDIA_CONCURRENT_READS: &str = "SCOPE_MEDIA_MAX_CONCURRENT_READS";

const DEFAULT_BLOCKING_OPERATIONS: usize = 4;
const DEFAULT_CONCURRENT_UPLOADS: usize = 4;
const DEFAULT_CONCURRENT_READS: usize = 8;

pub struct Settings {
    pub(crate) database_url: String,
    pub(crate) storage: MediaStorageSettings,
    pub(crate) grant_public_key_pem: String,
    pub(crate) max_blocking_operations: usize,
    pub(crate) max_concurrent_uploads: usize,
    pub(crate) max_concurrent_reads: usize,
    pub(crate) allowed_origin: Option<String>,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        let storage = MediaStorageSettings::from_env()?;
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
            storage,
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

fn validate_origin(origin: String) -> anyhow::Result<String> {
    let parsed = scope_service_config::ServiceEndpoint::parse_origin(&origin)?;
    anyhow::ensure!(
        parsed.as_str() == origin,
        "{MEDIA_ALLOWED_ORIGIN} must be an exact canonical origin"
    );
    Ok(origin)
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
