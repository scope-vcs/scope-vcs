use super::{
    cache_client::{CacheDownloadError, copy_hashed, validate_cache_size},
    source::{MAX_SOURCE_BYTES, retryable_source_status},
    *,
};
use std::{net::TcpListener, sync::mpsc};

#[test]
fn source_download_retries_transient_failures_and_installs_only_verified_bytes() {
    let body = b"verified source bundle";
    let digest = hex::encode(Sha256::digest(body));
    for first_response in [
        source_response("503 Service Unavailable", b"", "source-a", &digest, None),
        source_response("200 OK", b"partial", "source-a", &digest, Some(100)),
    ] {
        let (client, requests, server) = raw_test_client(vec![
            first_response,
            source_response("200 OK", body, "source-a", &digest, None),
        ]);
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("source.bundle");
        client.download_source("source-a", &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), body);
        assert_eq!(requests.try_iter().count(), 2);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
        server.join().unwrap();
    }
}

#[test]
fn source_download_rejects_invalid_sources_and_local_failures_without_retry_or_partial_files() {
    let body = b"verified source bundle";
    let digest = hex::encode(Sha256::digest(body));
    let wrong_digest = hex::encode(Sha256::digest(b"different bytes"));
    for (digest, advertised_length, path, expected_error) in [
        (
            wrong_digest.as_str(),
            None,
            "source.bundle",
            "downloaded source bytes do not match response digest".to_string(),
        ),
        (
            digest.as_str(),
            None,
            "missing/source.bundle",
            "create source bundle".to_string(),
        ),
        (
            digest.as_str(),
            Some(MAX_SOURCE_BYTES + 1),
            "source.bundle",
            format!("run source exceeds {MAX_SOURCE_BYTES} bytes"),
        ),
    ] {
        let (client, requests, server) = raw_test_client(vec![source_response(
            "200 OK",
            body,
            "source-a",
            digest,
            advertised_length,
        )]);
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join(path);
        let error = client
            .download_source("source-a", &destination)
            .unwrap_err();
        assert!(error.to_string().contains(&expected_error), "{error:#}");
        assert!(!destination.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        assert_eq!(requests.try_iter().count(), 1);
        server.join().unwrap();
    }
}

#[test]
fn source_download_retry_statuses_are_narrow() {
    for status in [429, 502, 503, 504] {
        assert!(retryable_source_status(
            StatusCode::from_u16(status).unwrap()
        ));
    }
    for status in [400, 401, 403, 404, 408, 409, 500, 501, 505] {
        assert!(!retryable_source_status(
            StatusCode::from_u16(status).unwrap()
        ));
    }
}

#[test]
fn cache_download_limit_is_one_gibibyte() {
    assert!(validate_cache_size(MAX_CACHE_OBJECT_BYTES).is_ok());
    let error = validate_cache_size(MAX_CACHE_OBJECT_BYTES + 1).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("cache exceeds {MAX_CACHE_OBJECT_BYTES} bytes")
    );
}

#[test]
fn cache_integrity_failures_are_invalid_not_transport_errors() {
    let mut source = &b"different"[..];
    let mut destination = Vec::new();
    let error = copy_hashed(&mut source, &mut destination, 9, "wrong").unwrap_err();
    assert!(matches!(error, CacheDownloadError::Invalid(_)));
}

#[test]
fn append_log_treats_only_rate_limiting_as_truncation() {
    let (client, requests, server) = test_client(&[
        ("429 Too Many Requests", ""),
        ("408 Request Timeout", ""),
        ("500 Internal Server Error", ""),
        ("409 Conflict", ""),
    ]);

    assert_eq!(
        client.append_log(2, 7, "first").unwrap(),
        AppendLogOutcome::Truncated
    );
    assert!(matches!(
        client.append_log(2, 7, "first"),
        Err(AppendLogError::Retryable(_))
    ));
    assert!(matches!(
        client.append_log(2, 7, "first"),
        Err(AppendLogError::Retryable(_))
    ));
    assert!(matches!(
        client.append_log(2, 7, "first"),
        Err(AppendLogError::Fatal(_))
    ));
    for _ in 0..4 {
        requests.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    server.join().unwrap();
}

#[test]
fn completion_requests_report_the_accumulated_truncation() {
    let status =
        r#"{"state":"succeeded","cancellation_requested":false,"lease_expires_at_unix":0}"#;
    let (client, requests, server) = test_client(&[("200 OK", status), ("200 OK", status)]);

    client.complete_step(3, 0, true).unwrap();
    client.complete_succeeded(true).unwrap();

    let step_request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
    let attempt_request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(step_request.starts_with("POST /v1/runtime-protocol/attempts/test/steps/3/complete"));
    assert!(attempt_request.starts_with("POST /v1/runtime-protocol/attempts/test/complete"));
    assert_eq!(request_json(&step_request)["logs_truncated"], true);
    assert_eq!(request_json(&attempt_request)["logs_truncated"], true);
    server.join().unwrap();
}

fn raw_test_client(
    responses: Vec<Vec<u8>>,
) -> (
    RuntimeClient,
    mpsc::Receiver<String>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (request_sender, requests) = mpsc::channel();
    let server = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            request_sender.send(read_request(&mut stream)).unwrap();
            stream.write_all(&response).unwrap();
        }
    });
    let client = RuntimeClient {
        client: Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap(),
        api_url: format!("http://{address}"),
        attempt_id: "test".to_string(),
        attempt_token: Arc::new(Mutex::new(Some("token".to_string()))),
        cache_access: Arc::new(Mutex::new(None)),
        cache_keys: Arc::new(Mutex::new(Vec::new())),
        heartbeat_lock: Arc::new(Mutex::new(())),
        heartbeat_started: None,
    };
    (client, requests, server)
}

fn source_response(
    status: &str,
    body: &[u8],
    identity: &str,
    digest: &str,
    advertised_length: Option<u64>,
) -> Vec<u8> {
    let content_length = advertised_length.unwrap_or(body.len() as u64);
    let mut response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Length: {content_length}\r\n\
         x-scope-source-identity: {identity}\r\n\
         x-scope-source-sha256: {digest}\r\n\
         Connection: close\r\n\r\n"
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

#[test]
fn heartbeats_cannot_replace_a_new_keyed_grant_with_an_older_empty_grant() {
    const SYNC_TIMEOUT: Duration = Duration::from_secs(5);
    const BLOCKED_OBSERVATION: Duration = Duration::from_secs(1);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (heartbeat_started_sender, heartbeat_started_receiver) = mpsc::channel();
    let (first_request_sender, first_request_receiver) = mpsc::channel();
    let (release_first_sender, release_first_receiver) = mpsc::channel();
    let (second_request_sender, second_request_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        first_request_sender.send(read_request(&mut first)).unwrap();
        let first_response = thread::spawn(move || {
            release_first_receiver.recv().unwrap();
            write_heartbeat_response(&mut first, "empty-grant");
        });

        let (mut second, _) = listener.accept().unwrap();
        second_request_sender
            .send(read_request(&mut second))
            .unwrap();
        write_heartbeat_response(&mut second, "keyed-grant");
        first_response.join().unwrap();
    });
    let client = RuntimeClient {
        client: Client::builder().timeout(SYNC_TIMEOUT).build().unwrap(),
        api_url: format!("http://{address}"),
        attempt_id: "test".to_string(),
        attempt_token: Arc::new(Mutex::new(Some("token".to_string()))),
        cache_access: Arc::new(Mutex::new(Some(CacheAccess {
            endpoint: "http://cache.invalid".to_string(),
            grant: "claim-grant".to_string(),
        }))),
        cache_keys: Arc::new(Mutex::new(Vec::new())),
        heartbeat_lock: Arc::new(Mutex::new(())),
        heartbeat_started: Some(heartbeat_started_sender),
    };

    let first_client = client.clone();
    let first_heartbeat = thread::spawn(move || first_client.heartbeat().unwrap());
    heartbeat_started_receiver
        .recv_timeout(SYNC_TIMEOUT)
        .unwrap();
    let first_request = first_request_receiver.recv_timeout(SYNC_TIMEOUT).unwrap();
    let material = AttemptCacheKeyMaterial {
        cache_name: "cargo".to_string(),
        compatibility_inputs_digest: "a".repeat(64),
        exact_inputs_digest: "b".repeat(64),
    };
    let second_client = client.clone();
    let authorization =
        thread::spawn(move || second_client.authorize_cache_keys(vec![material]).unwrap());
    heartbeat_started_receiver
        .recv_timeout(SYNC_TIMEOUT)
        .unwrap();
    assert!(
        matches!(
            second_request_receiver.recv_timeout(BLOCKED_OBSERVATION),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "a second heartbeat reached the server while the first remained in flight"
    );
    release_first_sender.send(()).unwrap();
    first_heartbeat.join().unwrap();
    authorization.join().unwrap();
    let second_request = second_request_receiver.recv_timeout(SYNC_TIMEOUT).unwrap();
    server.join().unwrap();

    assert_eq!(
        request_json(&first_request)["cache_keys"],
        serde_json::json!([])
    );
    assert_eq!(
        request_json(&second_request)["cache_keys"][0]["cache_name"],
        "cargo"
    );
    assert_eq!(
        client.cache_access.lock().unwrap().as_ref().unwrap().grant,
        "keyed-grant"
    );
}

fn test_client(
    responses: &[(&'static str, &'static str)],
) -> (
    RuntimeClient,
    mpsc::Receiver<String>,
    thread::JoinHandle<()>,
) {
    raw_test_client(responses.iter().map(|(status, body)| {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ).into_bytes()
    }).collect())
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer).unwrap();
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
            continue;
        };
        let header_end = header_end + 4;
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                    .map(str::to_owned)
            })
            .and_then(|length| length.parse::<usize>().ok())
            .unwrap_or(0);
        if request.len() >= header_end + content_length {
            return String::from_utf8(request).unwrap();
        }
    }
}

fn request_json(request: &str) -> serde_json::Value {
    let (_, body) = request.split_once("\r\n\r\n").unwrap();
    serde_json::from_str(body).unwrap()
}

fn write_heartbeat_response(stream: &mut std::net::TcpStream, grant: &str) {
    let body = serde_json::json!({
        "status": {
            "state": "succeeded",
            "cancellation_requested": false,
            "lease_expires_at_unix": 1
        },
        "cache_grant": grant
    })
    .to_string();
    write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
}
