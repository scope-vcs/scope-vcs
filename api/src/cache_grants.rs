#[cfg(any(test, feature = "local-dev"))]
use crate::grant_signing::TEST_PRIVATE_KEY;
use crate::grant_signing::{GrantSigner, required_env};
use scope_cache_contract::{AuthorizedCache, SignedCacheGrantClaims};
#[cfg(test)]
use scope_cache_domain::CacheDigest;
use scope_cache_domain::RepositoryId;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct CacheGrantIssuer {
    endpoint: Arc<str>,
    backend: Arc<str>,
    signer: GrantSigner,
}

impl CacheGrantIssuer {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Self::new(
            required_env("SCOPE_CACHE_URL")?,
            required_env("SCOPE_CACHE_BACKEND")?,
            required_env("SCOPE_CACHE_GRANT_PRIVATE_KEY")?,
        )
    }

    fn new(endpoint: String, backend: String, private_key_pem: String) -> anyhow::Result<Self> {
        let endpoint = scope_service_runtime::http::ServiceEndpoint::parse(&endpoint)
            .map_err(|error| anyhow::anyhow!("SCOPE_CACHE_URL: {error}"))?;
        if backend.is_empty()
            || backend.len() > 64
            || backend.starts_with('-')
            || backend.ends_with('-')
            || backend.contains("--")
            || !backend
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            anyhow::bail!(
                "SCOPE_CACHE_BACKEND must contain lowercase letters, digits, or single hyphens"
            );
        }
        Ok(Self {
            endpoint: Arc::from(endpoint.as_str()),
            backend: Arc::from(backend),
            signer: GrantSigner::from_ed25519_pem(&private_key_pem)?,
        })
    }

    pub(crate) fn issue(
        &self,
        attempt_id: String,
        repository_id: RepositoryId,
        allowed_caches: Vec<AuthorizedCache>,
        expires_at_unix: u64,
    ) -> anyhow::Result<String> {
        self.signer.sign(&SignedCacheGrantClaims {
            attempt_id,
            repository_id,
            allowed_caches,
            backend: self.backend.to_string(),
            expires_at_unix,
        })
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    #[cfg(any(test, feature = "local-dev"))]
    pub(crate) fn test() -> Self {
        Self::new(
            "http://127.0.0.1:8082".to_string(),
            "test-local".to_string(),
            TEST_PRIVATE_KEY.to_string(),
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grant_signing::TEST_PUBLIC_KEY;
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};

    #[test]
    fn configured_endpoint_is_validated_and_canonicalized_before_issuance() {
        for endpoint in [
            "http://127.0.0.1.example.net",
            "http://127.0.0.1@remote.example",
            "https://",
        ] {
            assert!(
                CacheGrantIssuer::new(endpoint.into(), "test".into(), TEST_PRIVATE_KEY.into())
                    .is_err()
            );
        }
        let issuer = CacheGrantIssuer::new(
            "https://EXAMPLE.com:443/cache/".into(),
            "test".into(),
            TEST_PRIVATE_KEY.into(),
        )
        .unwrap();
        assert_eq!(issuer.endpoint(), "https://example.com/cache");
    }

    #[test]
    fn grant_is_signed_with_the_dedicated_ed25519_key() {
        let issuer = CacheGrantIssuer::test();
        let token = issuer
            .issue(
                "attempt-1".to_string(),
                RepositoryId::parse("repo-1").unwrap(),
                vec![AuthorizedCache {
                    exact_identity_digest: CacheDigest::parse("a".repeat(64)).unwrap(),
                    compatibility_group_digest: CacheDigest::parse("b".repeat(64)).unwrap(),
                }],
                100,
            )
            .unwrap();
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.required_spec_claims.clear();
        validation.validate_exp = false;
        let claims = decode::<SignedCacheGrantClaims>(
            &token,
            &DecodingKey::from_ed_pem(TEST_PUBLIC_KEY.as_bytes()).unwrap(),
            &validation,
        )
        .unwrap()
        .claims;
        assert_eq!(claims.repository_id.as_str(), "repo-1");
        assert_eq!(claims.attempt_id, "attempt-1");
        assert_eq!(claims.backend, "test-local");
        assert_eq!(claims.expires_at_unix, 100);
    }
}
