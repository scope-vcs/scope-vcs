use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use scope_api_contract::attachments::{
    RequestAttachmentMediaGrantClaims, RequestAttachmentMediaGrantMethod,
    RequestAttachmentMediaTarget, RequestAttachmentUploadGrantClaims,
};
use std::sync::Arc;

const MEDIA_GRANT_LIFETIME_SECONDS: u64 = 300;

#[derive(Clone)]
pub(crate) struct MediaGrantIssuer {
    endpoint: Arc<str>,
    key: Arc<EncodingKey>,
}

impl MediaGrantIssuer {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Self::new(
            required_env("SCOPE_MEDIA_PUBLIC_URL")?,
            required_env("SCOPE_MEDIA_GRANT_PRIVATE_KEY")?,
        )
    }

    fn new(endpoint: String, private_key_pem: String) -> anyhow::Result<Self> {
        let endpoint = scope_service_config::ServiceEndpoint::parse_origin(&endpoint)
            .map_err(|error| anyhow::anyhow!("SCOPE_MEDIA_PUBLIC_URL: {error}"))?;
        Ok(Self {
            endpoint: Arc::from(endpoint.as_str()),
            key: Arc::new(EncodingKey::from_ed_pem(private_key_pem.as_bytes())?),
        })
    }

    pub(crate) fn expires_at(&self, now: u64) -> anyhow::Result<u64> {
        now.checked_add(MEDIA_GRANT_LIFETIME_SECONDS)
            .ok_or_else(|| anyhow::anyhow!("media grant expiry overflow"))
    }

    pub(crate) fn issue_upload(
        &self,
        claims: &RequestAttachmentUploadGrantClaims,
    ) -> anyhow::Result<String> {
        Ok(encode(&Header::new(Algorithm::EdDSA), claims, &self.key)?)
    }

    pub(crate) fn issue_media(
        &self,
        claims: &RequestAttachmentMediaGrantClaims,
    ) -> anyhow::Result<String> {
        anyhow::ensure!(
            claims.method == RequestAttachmentMediaGrantMethod::Get,
            "media grants only authorize reading attachments"
        );
        Ok(encode(&Header::new(Algorithm::EdDSA), claims, &self.key)?)
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn media_url(
        &self,
        attachment_id: &str,
        target: &RequestAttachmentMediaTarget,
        token: &str,
    ) -> anyhow::Result<String> {
        let mut url = url::Url::parse(self.endpoint())?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|()| anyhow::anyhow!("media URL cannot contain path segments"))?;
            segments.extend(["v1", "attachments", attachment_id]);
            match target {
                RequestAttachmentMediaTarget::Original => {
                    segments.push("original");
                }
                RequestAttachmentMediaTarget::Derivative { derivative_id } => {
                    segments.extend(["derivatives", derivative_id]);
                }
            }
        }
        url.query_pairs_mut().append_pair("grant", token);
        Ok(url.to_string())
    }

    #[cfg(any(test, feature = "local-dev", feature = "test-support"))]
    pub(crate) fn test() -> Self {
        Self::new("http://127.0.0.1:8083".into(), TEST_PRIVATE_KEY.into()).unwrap()
    }

    #[cfg(feature = "local-dev")]
    pub(crate) fn local() -> anyhow::Result<Self> {
        match (
            crate::config::non_empty_env("SCOPE_MEDIA_PUBLIC_URL"),
            crate::config::non_empty_env("SCOPE_MEDIA_GRANT_PRIVATE_KEY"),
        ) {
            (Some(endpoint), Some(key)) => Self::new(endpoint, key),
            (None, None) => Ok(Self::test()),
            _ => anyhow::bail!("local media configuration requires both URL and signing key"),
        }
    }
}

fn required_env(name: &str) -> anyhow::Result<String> {
    crate::config::non_empty_env(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

#[cfg(any(test, feature = "local-dev", feature = "test-support"))]
const TEST_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0\n-----END PRIVATE KEY-----\n";

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation, decode};

    #[test]
    fn upload_grant_preserves_exact_attachment_and_viewer_scope() {
        let issuer = MediaGrantIssuer::test();
        let claims = RequestAttachmentUploadGrantClaims {
            attachment_id: "attachment-a".into(),
            repository_id: "repo-a".into(),
            request_id: "request-a".into(),
            uploader_user_id: "uploader-a".into(),
            upload_id: "upload-a".into(),
            expires_at_unix: issuer.expires_at(10).unwrap(),
        };
        let token = issuer.issue_upload(&claims).unwrap();
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.required_spec_claims.clear();
        validation.validate_exp = false;
        let decoded = decode::<RequestAttachmentUploadGrantClaims>(
            &token,
            &DecodingKey::from_ed_pem(crate::cache_grants::TEST_PUBLIC_KEY.as_bytes()).unwrap(),
            &validation,
        )
        .unwrap()
        .claims;
        assert_eq!(decoded.attachment_id, "attachment-a");
        assert_eq!(decoded.uploader_user_id, "uploader-a");
        assert_eq!(decoded.upload_id, "upload-a");
        assert_eq!(decoded.expires_at_unix, 310);
    }

    #[test]
    fn origin_validation_rejects_credentials_and_loopback_lookalikes() {
        for invalid in [
            "http://127.0.0.1.example.com",
            "http://example.com",
            "https://user:secret@example.com",
            "https://example.com/path",
            "https://example.com?query=true",
            "https://example.com#fragment",
        ] {
            assert!(MediaGrantIssuer::new(invalid.into(), TEST_PRIVATE_KEY.into()).is_err());
        }
    }

    #[test]
    fn media_paths_cannot_turn_ids_into_query_or_path_injection() {
        let issuer = MediaGrantIssuer::test();
        let url = issuer
            .media_url(
                "attachment/a?b",
                &RequestAttachmentMediaTarget::Derivative {
                    derivative_id: "preview/a".into(),
                },
                "signed-token",
            )
            .unwrap();
        assert_eq!(
            url,
            "http://127.0.0.1:8083/v1/attachments/attachment%2Fa%3Fb/derivatives/preview%2Fa?grant=signed-token"
        );
    }
}
