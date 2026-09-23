use crate::{S3Settings, objects::ObjectStoreError};
use aws_credential_types::Credentials;
use aws_sigv4::{
    http_request::{
        PayloadChecksumKind, PercentEncodingMode, SignableBody, SignableRequest, SignatureLocation,
        SigningSettings, UriPathNormalizationMode, sign,
    },
    sign::v4,
};
use aws_smithy_http::label::{EncodingStrategy, fmt_string};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use std::time::{Duration, SystemTime};
use url::Url;

/// Signs time-limited URLs that let a client read or write an object directly, without the
/// object ever passing through a Scope service.
#[derive(Clone)]
pub struct S3Presigner {
    endpoint: String,
    bucket: String,
    region: String,
    credentials: Credentials,
    force_path_style: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresignedRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl S3Presigner {
    pub fn new(settings: &S3Settings) -> Self {
        Self {
            endpoint: settings.endpoint.trim_end_matches('/').to_string(),
            bucket: settings.bucket.clone(),
            region: settings.region.clone(),
            credentials: Credentials::new(
                settings.access_key_id.clone(),
                settings.secret_access_key.clone(),
                None,
                None,
                "scope-storage",
            ),
            force_path_style: settings.force_path_style,
        }
    }

    pub fn presign(
        &self,
        method: &str,
        key: &str,
        expires_seconds: u32,
    ) -> Result<String, ObjectStoreError> {
        Ok(self.presign_request(method, key, expires_seconds, &[])?.url)
    }

    pub fn presign_checksum_bound_put(
        &self,
        key: &str,
        expires_seconds: u32,
        checksum_sha256: &str,
        content_length: u64,
    ) -> Result<PresignedRequest, ObjectStoreError> {
        if checksum_sha256.len() != 64
            || !checksum_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ObjectStoreError::internal_message(
                "checksum must be 64 lowercase hexadecimal characters",
            ));
        }
        let checksum_bytes = hex::decode(checksum_sha256).map_err(|_| {
            ObjectStoreError::internal_message("checksum must be valid hexadecimal")
        })?;
        let checksum_base64 = BASE64.encode(checksum_bytes);
        let content_length = content_length.to_string();
        self.presign_request(
            "PUT",
            key,
            expires_seconds,
            &[
                ("content-length", content_length.as_str()),
                ("x-amz-checksum-sha256", checksum_base64.as_str()),
                ("x-amz-meta-scope-sha256", checksum_sha256),
            ],
        )
    }

    fn presign_request(
        &self,
        method: &str,
        key: &str,
        expires_seconds: u32,
        request_headers: &[(&str, &str)],
    ) -> Result<PresignedRequest, ObjectStoreError> {
        if !matches!(method, "GET" | "HEAD" | "PUT")
            || expires_seconds == 0
            || expires_seconds > 3600
        {
            return Err(ObjectStoreError::internal_message(
                "invalid presigned object request",
            ));
        }
        let mut request = http::Request::builder()
            .method(method)
            .uri(self.request_url(Some(key))?.as_str());
        for (name, value) in request_headers {
            request = request.header(*name, *value);
        }
        self.sign_request(
            request
                .body(SignableBody::UnsignedPayload)
                .map_err(ObjectStoreError::internal)?,
            Some(Duration::from_secs(u64::from(expires_seconds))),
            SystemTime::now(),
        )
    }

    fn request_url(&self, key: Option<&str>) -> Result<Url, ObjectStoreError> {
        let mut url = Url::parse(&self.endpoint).map_err(ObjectStoreError::internal)?;
        if !self.force_path_style {
            let host = url
                .host_str()
                .ok_or_else(|| ObjectStoreError::internal_message("invalid bucket endpoint"))?;
            url.set_host(Some(&format!("{}.{}", self.bucket, host)))
                .map_err(ObjectStoreError::internal)?;
        }
        let mut path = url.path().trim_end_matches('/').to_string();
        if self.force_path_style {
            path.push('/');
            path.push_str(&fmt_string(&self.bucket, EncodingStrategy::Default));
        }
        if let Some(key) = key {
            path.push('/');
            path.push_str(&fmt_string(key, EncodingStrategy::Greedy));
        }
        url.set_path(&path);
        Ok(url)
    }

    fn sign_request(
        &self,
        mut request: http::Request<SignableBody<'_>>,
        expires_in: Option<Duration>,
        now: SystemTime,
    ) -> Result<PresignedRequest, ObjectStoreError> {
        // S3 signs the encoded object path as sent, without normalization or a second encoding.
        let mut settings = SigningSettings::default();
        settings.percent_encoding_mode = PercentEncodingMode::Single;
        settings.uri_path_normalization_mode = UriPathNormalizationMode::Disabled;
        settings.expires_in = expires_in;
        if expires_in.is_some() {
            settings.signature_location = SignatureLocation::QueryParams;
        } else {
            settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
        }
        let identity = self.credentials.clone().into();
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name("s3")
            .time(now)
            .settings(settings)
            .build()
            .map_err(ObjectStoreError::internal)?
            .into();
        let headers = request
            .headers()
            .iter()
            .map(|(name, value)| value.to_str().map(|value| (name.as_str(), value)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(ObjectStoreError::internal)?;
        let signable = SignableRequest::new(
            request.method().as_str(),
            request.uri().to_string(),
            headers.into_iter(),
            request.body().clone(),
        )
        .map_err(ObjectStoreError::internal)?;
        let (instructions, _) = sign(signable, &params)
            .map_err(ObjectStoreError::internal)?
            .into_parts();
        instructions.apply_to_request_http1x(&mut request);
        Ok(PresignedRequest {
            url: request.uri().to_string(),
            headers: request
                .headers()
                .iter()
                .map(|(name, value)| {
                    value
                        .to_str()
                        .map(|value| (name.to_string(), value.to_string()))
                })
                .collect::<Result<_, _>>()
                .map_err(ObjectStoreError::internal)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presigner_produces_bounded_path_style_urls_without_exposing_the_secret() {
        let mut settings = S3Settings::new(
            "https://storage.example".into(),
            "scope-bucket".into(),
            "us-test-1".into(),
            "test-access".into(),
            "test-secret".into(),
        );
        settings.force_path_style = true;
        let url = S3Presigner::new(&settings)
            .presign("PUT", "repos/repo-1/objects/sha256/abc", 900)
            .unwrap();

        assert!(
            url.starts_with(
                "https://storage.example/scope-bucket/repos/repo-1/objects/sha256/abc?"
            )
        );
        assert!(url.contains("X-Amz-Expires=900"));
        assert!(url.contains("X-Amz-Credential=test-access%2F"));
        assert!(url.contains("X-Amz-Signature="));
        assert!(!url.contains("test-secret"));
        assert!(
            S3Presigner::new(&settings)
                .presign("DELETE", "key", 900)
                .is_err()
        );
        assert!(
            S3Presigner::new(&settings)
                .presign("GET", "key", 3_601)
                .is_err()
        );
    }

    #[test]
    fn checksum_bound_put_signs_and_returns_the_required_metadata_header() {
        let mut settings = S3Settings::new(
            "https://storage.example".into(),
            "scope-cache".into(),
            "us-east-1".into(),
            "access".into(),
            "secret".into(),
        );
        settings.force_path_style = true;
        let checksum = "a".repeat(64);
        let request = S3Presigner::new(&settings)
            .presign_checksum_bound_put("repos/repo-1/objects/sha256/abc", 900, &checksum, 42)
            .unwrap();

        assert!(request.url.contains(
            "X-Amz-SignedHeaders=content-length%3Bhost%3Bx-amz-checksum-sha256%3Bx-amz-meta-scope-sha256"
        ));
        assert_eq!(
            request.headers,
            vec![
                ("content-length".to_string(), "42".to_string()),
                (
                    "x-amz-checksum-sha256".to_string(),
                    "qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqo=".to_string(),
                ),
                ("x-amz-meta-scope-sha256".to_string(), checksum),
            ]
        );
        assert!(!request.url.contains("secret"));
    }

    #[test]
    fn presigner_supports_head_and_rejects_invalid_checksums() {
        let settings = S3Settings::new(
            "https://storage.example".into(),
            "scope-cache".into(),
            "us-east-1".into(),
            "access".into(),
            "secret".into(),
        );
        let signer = S3Presigner::new(&settings);

        assert!(signer.presign("HEAD", "object", 60).is_ok());
        assert!(
            signer
                .presign_checksum_bound_put("object", 60, "not-a-checksum", 42)
                .is_err()
        );
    }

    #[test]
    fn addressing_preserves_endpoint_prefix_port_and_encoded_object_key() {
        let mut settings = S3Settings::new(
            "https://storage.example:9443/base/".into(),
            "scope-bucket".into(),
            "us-test-1".into(),
            "test-access".into(),
            "test-secret".into(),
        );
        for (path_style, expected) in [
            (false, "https://scope-bucket.storage.example:9443/base"),
            (true, "https://storage.example:9443/base/scope-bucket"),
        ] {
            settings.force_path_style = path_style;
            let signer = S3Presigner::new(&settings);
            assert_eq!(signer.request_url(None).unwrap().as_str(), expected);
            assert_eq!(
                signer
                    .request_url(Some("objects/a b+c?#%//tail"))
                    .unwrap()
                    .as_str(),
                format!("{expected}/objects/a%20b%2Bc%3F%23%25//tail"),
            );
        }
    }

    #[test]
    fn signatures_match_fixed_vectors_and_bind_every_upload_constraint() {
        let mut settings = S3Settings::new(
            "https://storage.example:9443/base".into(),
            "scope-bucket".into(),
            "us-test-1".into(),
            "test-access".into(),
            "test-secret".into(),
        );
        settings.force_path_style = true;
        let signer = S3Presigner::new(&settings);
        let key = "objects/a b+c?#%";
        let uri = signer.request_url(Some(key)).unwrap();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let request = http::Request::builder()
            .method("PUT")
            .uri(uri.as_str())
            .body(SignableBody::Bytes(b"stored payload"))
            .unwrap();
        let signed = signer.sign_request(request, None, now).unwrap();
        let authorization = signed
            .headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .unwrap();
        // Fixed vectors calculated independently from the SigV4 canonical request and HMAC steps.
        assert!(authorization.1.ends_with(
            "Signature=bfe226c6525f3f73884f949f113f4e110e00d5aa240f9b7744e82b32b8dcbba0"
        ));
        let upload = signer
            .presign_checksum_bound_put(key, 900, &"a".repeat(64), 42)
            .unwrap();
        let presign = |headers: &[(String, String)]| {
            let mut request = http::Request::builder().method("PUT").uri(uri.as_str());
            for (name, value) in headers {
                request = request.header(name, value);
            }
            signer
                .sign_request(
                    request.body(SignableBody::UnsignedPayload).unwrap(),
                    Some(Duration::from_secs(900)),
                    now,
                )
                .unwrap()
                .url
        };
        let original = presign(&upload.headers);
        assert!(original.ends_with(
            "X-Amz-Signature=33a1381bcaa13ea9cbee2997ae08046c25dda313590c2353c4d6d5a87dd45bad"
        ));
        for index in 0..upload.headers.len() {
            let mut changed = upload.headers.clone();
            changed[index].1.push('0');
            assert_ne!(
                presign(&changed),
                original,
                "{} must be signed",
                changed[index].0
            );
        }
    }
}
