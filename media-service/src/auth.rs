use crate::error::ServiceError;
use axum::http::{HeaderMap, header::AUTHORIZATION};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use scope_api_contract::{
    RequestAttachmentMediaGrantClaims, RequestAttachmentMediaTarget,
    RequestAttachmentUploadGrantClaims,
};

pub(crate) struct GrantVerifier {
    key: DecodingKey,
}

impl GrantVerifier {
    pub(crate) fn new(public_key_pem: &str) -> anyhow::Result<Self> {
        if public_key_pem.trim().is_empty() {
            anyhow::bail!("media grant public key is required");
        }
        Ok(Self {
            key: DecodingKey::from_ed_pem(public_key_pem.as_bytes())?,
        })
    }

    pub(crate) fn verify_upload(
        &self,
        headers: &HeaderMap,
        upload_id: &str,
        now_unix: u64,
    ) -> Result<RequestAttachmentUploadGrantClaims, ServiceError> {
        let authorization = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| ServiceError::unauthorized("media upload grant is required"))?;
        let token = authorization
            .strip_prefix("Bearer ")
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| ServiceError::unauthorized("media upload grant is malformed"))?;
        let claims: RequestAttachmentUploadGrantClaims =
            self.decode(token, "media upload grant is invalid")?;
        if claims.attachment_id.is_empty()
            || !claims.allows_part(upload_id, &claims.attachment_id, now_unix)
        {
            return Err(ServiceError::unauthorized(
                "media upload grant is expired or targets another upload",
            ));
        }
        Ok(claims)
    }

    pub(crate) fn verify_media(
        &self,
        token: &str,
        attachment_id: &str,
        target: &RequestAttachmentMediaTarget,
        now_unix: u64,
    ) -> Result<RequestAttachmentMediaGrantClaims, ServiceError> {
        if token.trim().is_empty() {
            return Err(ServiceError::unauthorized("media grant is required"));
        }
        let claims: RequestAttachmentMediaGrantClaims =
            self.decode(token, "media grant is invalid")?;
        if !claims.allows(attachment_id, target, now_unix) {
            return Err(ServiceError::unauthorized(
                "media grant is expired or targets another object",
            ));
        }
        Ok(claims)
    }

    fn decode<T: serde::de::DeserializeOwned>(
        &self,
        token: &str,
        invalid_message: &'static str,
    ) -> Result<T, ServiceError> {
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.required_spec_claims.clear();
        validation.validate_exp = false;
        decode::<T>(token, &self.key, &validation)
            .map(|data| data.claims)
            .map_err(|_| ServiceError::unauthorized(invalid_message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use scope_api_contract::RequestAttachmentMediaGrantMethod;

    #[test]
    fn ed25519_grants_enforce_expiry_upload_and_media_targets() {
        let verifier = GrantVerifier::new(TEST_PUBLIC_KEY).unwrap();
        let upload = RequestAttachmentUploadGrantClaims {
            attachment_id: "att_1".to_string(),
            repository_id: "repo_1".to_string(),
            request_id: "req_1".to_string(),
            uploader_user_id: "user_1".to_string(),
            upload_id: "upload_1".to_string(),
            expires_at_unix: 100,
        };
        let upload_token = token(&upload);
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {upload_token}")).unwrap(),
        );
        assert!(verifier.verify_upload(&headers, "upload_1", 99).is_ok());
        assert!(verifier.verify_upload(&headers, "upload_2", 99).is_err());
        assert!(verifier.verify_upload(&headers, "upload_1", 100).is_err());

        let target = RequestAttachmentMediaTarget::Original;
        let media = RequestAttachmentMediaGrantClaims {
            attachment_id: "att_1".to_string(),
            repository_id: "repo_1".to_string(),
            request_id: "req_1".to_string(),
            viewer_user_id: Some("user_1".to_string()),
            method: RequestAttachmentMediaGrantMethod::Get,
            target: target.clone(),
            expires_at_unix: 100,
        };
        let media_token = token(&media);
        assert!(
            verifier
                .verify_media(&media_token, "att_1", &target, 99)
                .is_ok()
        );
        assert!(
            verifier
                .verify_media(
                    &media_token,
                    "att_1",
                    &RequestAttachmentMediaTarget::Derivative {
                        derivative_id: "der_1".to_string(),
                    },
                    99,
                )
                .is_err()
        );
        assert!(
            verifier
                .verify_media(&media_token, "att_1", &target, 100)
                .is_err()
        );
    }

    fn token<T: serde::Serialize>(claims: &T) -> String {
        encode(
            &Header::new(Algorithm::EdDSA),
            claims,
            &EncodingKey::from_ed_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap(),
        )
        .unwrap()
    }

    const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0\n-----END PRIVATE KEY-----\n";
    const TEST_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA2+Jj2UvNCvQiUPNYRgSi0cJSPiJI6Rs6D0UTeEpQVj8=\n-----END PUBLIC KEY-----\n";
}
