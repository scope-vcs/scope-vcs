use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct GrantSigner(Arc<EncodingKey>);

impl GrantSigner {
    pub(crate) fn from_ed25519_pem(private_key_pem: &str) -> anyhow::Result<Self> {
        Ok(Self(Arc::new(EncodingKey::from_ed_pem(
            private_key_pem.as_bytes(),
        )?)))
    }

    pub(crate) fn sign(&self, claims: &impl Serialize) -> anyhow::Result<String> {
        Ok(encode(&Header::new(Algorithm::EdDSA), claims, &self.0)?)
    }
}

pub(crate) fn required_env(name: &str) -> anyhow::Result<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

#[cfg(any(test, feature = "local-dev"))]
pub(crate) const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0\n-----END PRIVATE KEY-----\n";

#[cfg(test)]
pub(crate) const TEST_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA2+Jj2UvNCvQiUPNYRgSi0cJSPiJI6Rs6D0UTeEpQVj8=\n-----END PUBLIC KEY-----\n";
