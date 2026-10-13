use axum::http::HeaderMap;
use scope_api_contract::{
    RequestAttachmentMediaGrantClaims, RequestAttachmentMediaTarget,
    RequestAttachmentUploadGrantClaims,
};
use scope_service_runtime::{
    grants::{BearerError, GrantDecoder, bearer_token},
    http::ServiceError,
};

pub(crate) struct GrantVerifier {
    decoder: GrantDecoder,
}

impl GrantVerifier {
    pub(crate) fn new(public_key_pem: &str) -> anyhow::Result<Self> {
        Ok(Self {
            decoder: GrantDecoder::from_public_key_pem(public_key_pem)?,
        })
    }

    pub(crate) fn verify_upload(
        &self,
        headers: &HeaderMap,
        upload_id: &str,
        now_unix: u64,
    ) -> Result<RequestAttachmentUploadGrantClaims, ServiceError> {
        let token = bearer_token(headers).map_err(|error| match error {
            BearerError::Missing => ServiceError::unauthorized("media upload grant is required"),
            BearerError::Malformed => ServiceError::unauthorized("media upload grant is malformed"),
        })?;
        let claims: RequestAttachmentUploadGrantClaims = self
            .decoder
            .decode(token)
            .ok_or_else(|| ServiceError::unauthorized("media upload grant is invalid"))?;
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
        let claims: RequestAttachmentMediaGrantClaims = self
            .decoder
            .decode(token)
            .ok_or_else(|| ServiceError::unauthorized("media grant is invalid"))?;
        if !claims.allows(attachment_id, target, now_unix) {
            return Err(ServiceError::unauthorized(
                "media grant is expired or targets another object",
            ));
        }
        Ok(claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, header::AUTHORIZATION};
    use scope_service_runtime::grants::test_keys::{PUBLIC_KEY_PEM, sign};

    #[test]
    fn ed25519_grants_enforce_expiry_upload_and_media_targets() {
        let verifier = GrantVerifier::new(PUBLIC_KEY_PEM).unwrap();
        let upload = RequestAttachmentUploadGrantClaims {
            attachment_id: "att_1".to_string(),
            repository_id: "repo_1".to_string(),
            request_id: "req_1".to_string(),
            uploader_user_id: "user_1".to_string(),
            upload_id: "upload_1".to_string(),
            expires_at_unix: 100,
        };
        let upload_token = sign(&upload);
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
            target: target.clone(),
            expires_at_unix: 100,
        };
        let media_token = sign(&media);
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
}
