use super::{ObjectStore, ensure_object_size, object_too_large};
use crate::ObjectStoreError;
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
use reqwest::blocking::Client;
use std::{
    io::Read,
    time::{Duration, SystemTime},
};

const S3_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const S3_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct S3ObjectStoreSettings {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub force_path_style: bool,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl S3ObjectStoreSettings {
    pub fn new(
        endpoint: String,
        bucket: String,
        region: String,
        access_key_id: String,
        secret_access_key: String,
    ) -> Self {
        Self {
            endpoint,
            bucket,
            region,
            access_key_id,
            secret_access_key,
            force_path_style: false,
            connect_timeout: S3_CONNECT_TIMEOUT,
            request_timeout: S3_REQUEST_TIMEOUT,
        }
    }
}

pub struct S3ObjectStore {
    client: Option<Client>,
    signer: S3Presigner,
    request_timeout: Duration,
}

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
    pub fn new(settings: &S3ObjectStoreSettings) -> Self {
        Self {
            endpoint: settings.endpoint.trim_end_matches('/').to_string(),
            bucket: settings.bucket.clone(),
            region: settings.region.clone(),
            credentials: Credentials::new(
                settings.access_key_id.clone(),
                settings.secret_access_key.clone(),
                None,
                None,
                "scope-object-store",
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

    fn request_url(&self, key: Option<&str>) -> Result<reqwest::Url, ObjectStoreError> {
        let mut url = reqwest::Url::parse(&self.endpoint).map_err(ObjectStoreError::internal)?;
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

impl S3ObjectStore {
    pub fn new(settings: S3ObjectStoreSettings) -> Result<Self, ObjectStoreError> {
        Ok(Self {
            client: Some(
                Client::builder()
                    .connect_timeout(settings.connect_timeout)
                    .build()
                    .map_err(|error| {
                        ObjectStoreError::internal(format!("building object store client: {error}"))
                    })?,
            ),
            signer: S3Presigner::new(&settings),
            request_timeout: settings.request_timeout,
        })
    }

    fn send(
        &self,
        method: &str,
        key: Option<&str>,
        payload: Vec<u8>,
        max_bytes: Option<usize>,
    ) -> Result<Vec<u8>, ObjectStoreError> {
        let signed = self.signer.sign_request(
            http::Request::builder()
                .method(method)
                .uri(self.signer.request_url(key)?.as_str())
                .body(SignableBody::Bytes(&payload))
                .map_err(ObjectStoreError::internal)?,
            None,
            SystemTime::now(),
        )?;
        let client = self.client.as_ref().ok_or_else(|| {
            ObjectStoreError::internal_message("object store client is shut down")
        })?;
        let mut request = client
            .request(
                method.parse().map_err(ObjectStoreError::internal)?,
                &signed.url,
            )
            .timeout(self.request_timeout);
        if method == "PUT" {
            request = request.body(payload);
        }
        for (name, value) in signed.headers {
            request = request.header(name, value);
        }
        send_blocking_request(method, key, request, max_bytes)
    }
}

fn send_blocking_request(
    method: &str,
    key: Option<&str>,
    request: reqwest::blocking::RequestBuilder,
    max_bytes: Option<usize>,
) -> Result<Vec<u8>, ObjectStoreError> {
    let send = || {
        let label = key.unwrap_or("bucket");
        let response = request.send().map_err(|error| {
            ObjectStoreError::service_unavailable(format!(
                "object store {method} failed for {label}: {error}"
            ))
        })?;
        let status = response.status();
        if !status.is_success() {
            let message = format!("object store {method} failed for {label}: {status}");
            return Err(
                if key.is_some()
                    && matches!(method, "GET" | "HEAD")
                    && status == reqwest::StatusCode::NOT_FOUND
                {
                    ObjectStoreError::not_found(message)
                } else {
                    ObjectStoreError::service_unavailable(message)
                },
            );
        }
        read_response_body(response, label, max_bytes)
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(send)
    } else {
        send()
    }
}

fn read_response_body(
    mut response: reqwest::blocking::Response,
    key: &str,
    max_bytes: Option<usize>,
) -> Result<Vec<u8>, ObjectStoreError> {
    let Some(max_bytes) = max_bytes else {
        let mut body = Vec::new();
        response.read_to_end(&mut body).map_err(|error| {
            ObjectStoreError::service_unavailable(format!("reading object {key} failed: {error}"))
        })?;
        return Ok(body);
    };

    if let Some(content_length) = response.content_length()
        && content_length > max_bytes as u64
    {
        return Err(object_too_large(
            "read",
            key,
            usize::try_from(content_length).unwrap_or(usize::MAX),
            max_bytes,
        ));
    }

    let mut body = Vec::new();
    response
        .take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut body)
        .map_err(|error| {
            ObjectStoreError::service_unavailable(format!("reading object {key} failed: {error}"))
        })?;
    ensure_object_size("read", key, body.len(), max_bytes)?;
    Ok(body)
}

impl Drop for S3ObjectStore {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            // reqwest's blocking client owns runtime resources. This object is
            // process-lifetime state, so avoid async-context shutdown panics.
            std::mem::forget(client);
        }
    }
}

impl ObjectStore for S3ObjectStore {
    fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
        self.send("PUT", Some(key), bytes, None).map(|_| ())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        self.send("GET", Some(key), Vec::new(), None)
    }

    fn get_bounded(&self, key: &str, max_bytes: usize) -> Result<Vec<u8>, ObjectStoreError> {
        self.send("GET", Some(key), Vec::new(), Some(max_bytes))
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        self.send("DELETE", Some(key), Vec::new(), None).map(|_| ())
    }

    fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        self.send("HEAD", None, Vec::new(), None).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn s3_store_checks_bucket_with_signed_head_request() {
        let server = TestS3Server::start(vec![(vec![], None)]);
        let store = test_s3_store(&server.endpoint);

        store.readiness_check().unwrap();

        let request = server.recv();
        assert_eq!(request.method, "HEAD");
        assert_eq!(request.path, "/scope-bucket");
        assert_eq!(
            request.headers.get("host").map(String::as_str),
            Some(server.host.as_str())
        );
        assert_signed_s3_headers(&request);
    }

    #[test]
    fn s3_store_put_get_delete_use_signed_local_s3_compatible_requests() {
        let server = TestS3Server::start(vec![
            (vec![], None),
            (b"stored payload".to_vec(), None),
            (vec![], None),
        ]);
        let store = test_s3_store(&server.endpoint);
        let key = "objects/blob-1";

        store.put(key, b"stored payload".to_vec()).unwrap();
        assert_eq!(store.get(key).unwrap(), b"stored payload");
        store.delete(key).unwrap();

        for (method, body) in [
            ("PUT", b"stored payload".as_slice()),
            ("GET", b"".as_slice()),
            ("DELETE", b"".as_slice()),
        ] {
            let request = server.recv();
            assert_eq!(request.method, method);
            assert_eq!(request.path, "/scope-bucket/objects/blob-1");
            assert_eq!(request.body, body);
            assert_signed_s3_headers(&request);
        }
    }

    #[test]
    fn s3_store_bounded_get_rejects_declared_oversized_body_before_reading() {
        let server = TestS3Server::start(vec![(vec![], Some(5))]);
        let store = test_s3_store(&server.endpoint);

        let error = store.get_bounded("objects/too-large", 4).unwrap_err();

        assert_eq!(error.kind, crate::ObjectStoreErrorKind::PayloadTooLarge);
        assert!(error.message.contains("exceeds 4 bytes"));
        let request = server.recv();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/scope-bucket/objects/too-large");
        assert_signed_s3_headers(&request);
    }

    #[test]
    fn presigner_produces_bounded_path_style_urls_without_exposing_the_secret() {
        let mut settings = S3ObjectStoreSettings::new(
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
        let mut settings = S3ObjectStoreSettings::new(
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
        let settings = S3ObjectStoreSettings::new(
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
        let mut settings = S3ObjectStoreSettings::new(
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
        let mut settings = S3ObjectStoreSettings::new(
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

    #[test]
    fn bounded_get_limits_chunked_responses_without_content_length() {
        let server = TestS3Server::start_wire_responses(vec![
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nlarge\r\n0\r\n\r\n".to_vec(),
        ]);
        let error = test_s3_store(&server.endpoint)
            .get_bounded("object", 4)
            .unwrap_err();
        assert_eq!(error.kind, crate::ObjectStoreErrorKind::PayloadTooLarge);
    }

    #[test]
    fn request_timeout_covers_stalled_response_body() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            use std::io::Write as _;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
                .unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            // Keep the advertised byte pending until the client closes the timed-out request.
            let _ = stream.read(&mut [0]);
        });
        let mut store = test_s3_store(&endpoint);
        store.request_timeout = Duration::from_millis(50);
        let started = std::time::Instant::now();
        let error = store.get("stalled").unwrap_err();
        assert_eq!(error.kind, crate::ObjectStoreErrorKind::ServiceUnavailable);
        assert!(started.elapsed() < Duration::from_secs(1));
        server.join().unwrap();
    }

    #[test]
    fn s3_reads_distinguish_missing_objects_from_unavailable_storage() {
        use crate::ObjectStoreErrorKind::{NotFound, ServiceUnavailable};
        for (status, expected) in [
            ("404 Not Found", NotFound),
            ("403 Forbidden", ServiceUnavailable),
            ("503 Service Unavailable", ServiceUnavailable),
        ] {
            let server = TestS3Server::start_wire_responses(vec![
                format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .into_bytes(),
            ]);
            let error = test_s3_store(&server.endpoint).get("missing").unwrap_err();
            assert_eq!(error.kind, expected);
            assert!(error.message.contains(status));
        }
    }

    #[test]
    fn missing_bucket_is_an_unavailable_store() {
        let server = TestS3Server::start_wire_responses(vec![
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        ]);
        let error = test_s3_store(&server.endpoint)
            .readiness_check()
            .unwrap_err();
        assert_eq!(error.kind, crate::ObjectStoreErrorKind::ServiceUnavailable);
    }

    fn test_s3_store(endpoint: &str) -> S3ObjectStore {
        let mut settings = S3ObjectStoreSettings::new(
            endpoint.to_string(),
            "scope-bucket".to_string(),
            "us-test-1".to_string(),
            "test-access".to_string(),
            "test-secret".to_string(),
        );
        settings.force_path_style = true;
        settings.connect_timeout = Duration::from_secs(1);
        settings.request_timeout = Duration::from_secs(1);
        S3ObjectStore::new(settings).unwrap()
    }

    fn assert_signed_s3_headers(request: &CapturedRequest) {
        let authorization = request
            .headers
            .get("authorization")
            .expect("authorization header");
        assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=test-access/"));
        assert!(authorization.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date"));
        assert!(!authorization.contains("test-secret"));
        assert!(request.headers.contains_key("x-amz-content-sha256"));
        assert!(request.headers.contains_key("x-amz-date"));
    }

    #[derive(Debug)]
    struct CapturedRequest {
        method: String,
        path: String,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    }

    struct TestS3Server {
        endpoint: String,
        host: String,
        requests: std::sync::mpsc::Receiver<CapturedRequest>,
    }

    impl TestS3Server {
        fn start(responses: Vec<(Vec<u8>, Option<usize>)>) -> Self {
            Self::start_wire_responses(responses.into_iter().map(|(body, declared_length)| {
                let content_length = declared_length.unwrap_or(body.len());
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
                ).into_bytes();
                response.extend(body);
                response
            }).collect())
        }

        fn start_wire_responses(responses: Vec<Vec<u8>>) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let host = format!("127.0.0.1:{}", addr.port());
            let endpoint = format!("http://{host}");
            let (sender, requests) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                for response in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request = read_request(&mut stream);
                    sender.send(request).unwrap();
                    use std::io::Write as _;
                    stream.write_all(&response).unwrap();
                }
            });
            Self {
                endpoint,
                host,
                requests,
            }
        }

        fn recv(&self) -> CapturedRequest {
            self.requests
                .recv_timeout(Duration::from_secs(2))
                .expect("mock S3 request")
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> CapturedRequest {
        use std::io::{BufRead as _, Read as _};

        let mut reader = std::io::BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut request_parts = line.split_whitespace();
        let method = request_parts.next().unwrap().to_string();
        let path = request_parts.next().unwrap().to_string();
        let mut headers = BTreeMap::new();
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
        }
        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).unwrap();

        CapturedRequest {
            method,
            path,
            headers,
            body,
        }
    }
}
