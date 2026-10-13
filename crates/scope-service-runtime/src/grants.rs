use axum::http::{HeaderMap, header::AUTHORIZATION};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::de::DeserializeOwned;

pub struct GrantDecoder {
    key: DecodingKey,
    validation: Validation,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BearerError {
    Missing,
    Malformed,
}

impl GrantDecoder {
    pub fn from_public_key_pem(public_key_pem: &str) -> anyhow::Result<Self> {
        if public_key_pem.trim().is_empty() {
            anyhow::bail!("grant public key is required");
        }
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.required_spec_claims.clear();
        validation.validate_exp = false;
        Ok(Self {
            key: DecodingKey::from_ed_pem(public_key_pem.as_bytes())?,
            validation,
        })
    }

    pub fn decode<T: DeserializeOwned>(&self, token: &str) -> Option<T> {
        jsonwebtoken::decode::<T>(token, &self.key, &self.validation)
            .ok()
            .map(|data| data.claims)
    }
}

pub fn bearer_token(headers: &HeaderMap) -> Result<&str, BearerError> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(BearerError::Missing)?
        .strip_prefix("Bearer ")
        .filter(|token| !token.trim().is_empty())
        .ok_or(BearerError::Malformed)
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_keys {
    use jsonwebtoken::{Algorithm, EncodingKey, Header};

    pub const PUBLIC_KEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA2+Jj2UvNCvQiUPNYRgSi0cJSPiJI6Rs6D0UTeEpQVj8=\n-----END PUBLIC KEY-----\n";
    const PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0\n-----END PRIVATE KEY-----\n";

    pub fn sign<T: serde::Serialize>(claims: &T) -> String {
        jsonwebtoken::encode(
            &Header::new(Algorithm::EdDSA),
            claims,
            &EncodingKey::from_ed_pem(PRIVATE_KEY_PEM.as_bytes()).unwrap(),
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[derive(Debug, PartialEq, serde::Deserialize, serde::Serialize)]
    struct Claims {
        subject: String,
    }

    #[test]
    fn decodes_only_grants_signed_by_the_configured_key() {
        let decoder = GrantDecoder::from_public_key_pem(test_keys::PUBLIC_KEY_PEM).unwrap();
        let claims = Claims {
            subject: "upload_1".into(),
        };
        assert_eq!(
            decoder.decode::<Claims>(&test_keys::sign(&claims)),
            Some(claims)
        );
        assert_eq!(decoder.decode::<Claims>("not-a-grant"), None);
        assert!(GrantDecoder::from_public_key_pem(" ").is_err());
    }

    #[test]
    fn bearer_token_requires_a_non_empty_bearer_credential() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer_token(&headers), Err(BearerError::Missing));
        for malformed in ["Basic abc", "Bearer ", "Bearer   "] {
            headers.insert(AUTHORIZATION, HeaderValue::from_static(malformed));
            assert_eq!(bearer_token(&headers), Err(BearerError::Malformed));
        }
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer grant"));
        assert_eq!(bearer_token(&headers), Ok("grant"));
    }
}
