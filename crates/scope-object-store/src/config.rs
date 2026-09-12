//! Environment configuration for object storage.
use crate::{ObjectStoreError, S3ObjectStoreSettings};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

pub fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub fn required_env(name: &str) -> Result<String, ObjectStoreError> {
    nonempty_env(name)
        .ok_or_else(|| ObjectStoreError::internal_message(format!("{name} is required")))
}

impl S3ObjectStoreSettings {
    pub fn from_env(prefix: &str) -> Result<Self, ObjectStoreError> {
        let read = |suffix| required_env(&format!("{prefix}_{suffix}"));
        let endpoint = read("ENDPOINT")?;
        validate_endpoint(&endpoint)?;
        Ok(Self::new(
            endpoint,
            read("NAME")?,
            read("REGION")?,
            read("ACCESS_KEY_ID")?,
            read("SECRET_ACCESS_KEY")?,
        ))
    }
}

fn validate_endpoint(value: &str) -> Result<(), ObjectStoreError> {
    let url = reqwest::Url::parse(value).map_err(ObjectStoreError::internal)?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ObjectStoreError::internal_message(
            "object storage endpoint must be an HTTPS origin or an HTTP loopback origin",
        ));
    }
    Ok(())
}

pub fn encryption_key_from_env(name: &str) -> Result<[u8; 32], ObjectStoreError> {
    decode_encryption_key(name, &required_env(name)?)
}

fn decode_encryption_key(name: &str, encoded: &str) -> Result<[u8; 32], ObjectStoreError> {
    let decoded = BASE64.decode(encoded.trim()).map_err(|error| {
        ObjectStoreError::internal_message(format!("{name} must be base64: {error}"))
    })?;
    decoded.try_into().map_err(|_| {
        ObjectStoreError::internal_message(format!("{name} must decode to exactly 32 bytes"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_rejects_insecure_remote_and_loopback_lookalikes() {
        for endpoint in [
            "https://bucket.example",
            "http://localhost:9000",
            "http://127.0.0.1:9000",
            "http://[::1]:9000",
        ] {
            assert!(validate_endpoint(endpoint).is_ok(), "{endpoint}");
        }
        for endpoint in [
            "http://bucket.example",
            "http://127.0.0.1.attacker",
            "https://user@bucket.example",
            "https://bucket.example/path",
            "https://bucket.example?query",
        ] {
            assert!(validate_endpoint(endpoint).is_err(), "{endpoint}");
        }
        let mut password_endpoint = reqwest::Url::parse("https://bucket.example").unwrap();
        password_endpoint.set_password(Some("fixture")).unwrap();
        assert!(validate_endpoint(password_endpoint.as_str()).is_err());
    }
    #[test]
    fn key_decode_trims_and_requires_exactly_32_bytes() {
        assert_eq!(
            decode_encryption_key("KEY", &format!(" {}\n", BASE64.encode([7; 32]))).unwrap(),
            [7; 32]
        );
        assert!(decode_encryption_key("KEY", &BASE64.encode([7; 31])).is_err());
        assert!(decode_encryption_key("KEY", "invalid base64").is_err());
    }
}
