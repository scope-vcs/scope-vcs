use super::*;
use reqwest::StatusCode;
use std::{io::Write, net::TcpListener, thread};

#[test]
fn list_requests_sends_the_opaque_cursor_as_a_query_parameter() {
    let (api_url, server) = serve_once(StatusCode::OK, r#"{"requests":[],"next_cursor":null}"#);

    let response = list_requests(
        ApiSession::new(&http_client_builder().build().unwrap(), &api_url, "token"),
        "owner",
        "repo",
        Some("next page/+"),
    )
    .unwrap();

    assert!(response.requests.is_empty());
    assert!(response.next_cursor.is_none());
    let request = server.join().unwrap();
    assert!(
        request
            .lines()
            .next()
            .unwrap()
            .ends_with("?cursor=next+page%2F%2B HTTP/1.1"),
        "{request}"
    );
}

#[test]
fn request_named_filters_by_the_exact_name_in_one_request() {
    let (api_url, server) = serve_once(StatusCode::OK, r#"{"requests":[],"next_cursor":null}"#);

    let found = request_named(
        ApiSession::new(&http_client_builder().build().unwrap(), &api_url, "token"),
        "owner",
        "repo",
        "fix/a b",
    )
    .unwrap();

    assert!(found.is_none());
    let request = server.join().unwrap();
    assert!(
        request
            .lines()
            .next()
            .unwrap()
            .ends_with("/v1/repos/owner/repo/requests?name=fix%2Fa+b HTTP/1.1"),
        "{request}"
    );
}

#[test]
fn request_errors_surface_terminal_safe_message_instruction_and_reference() {
    let (api_url, server) = serve_once(
        StatusCode::CONFLICT,
        r#"{"code":"conflict","message":"request cannot be submitted\u001b[31m","instruction":"Retry later.\u001b[31m","error_reference":"err_0123456789abcdef0123456789abcdef\u001b[31m","retryable":false}"#,
    );

    let error = submit_request(
        ApiSession::new(&http_client_builder().build().unwrap(), &api_url, "token"),
        target(),
    )
    .unwrap_err()
    .to_string();

    assert_eq!(
        error,
        "request cannot be submitted [31m\nRetry later. [31m\nReference: err_0123456789abcdef0123456789abcdef [31m"
    );
    server.join().unwrap();
}

#[test]
fn malformed_error_bodies_use_a_scoped_status_fallback() {
    let (api_url, server) = serve_once(StatusCode::SERVICE_UNAVAILABLE, "upstream exploded");

    let error = merge_request(
        ApiSession::new(&http_client_builder().build().unwrap(), &api_url, "token"),
        target(),
        GitOid::try_from("a".repeat(40).as_str()).unwrap(),
    )
    .unwrap_err()
    .to_string();

    assert_eq!(
        error,
        "Scope is temporarily unavailable while trying to merge request req_one for owner/repo"
    );
    server.join().unwrap();
}

fn target() -> RequestTarget<'static> {
    RequestTarget {
        owner: "owner",
        repo: "repo",
        request_id: "req_one",
    }
}

fn serve_once(status: StatusCode, body: impl Into<String>) -> (String, thread::JoinHandle<String>) {
    let body = body.into();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = crate::test_support::read_http_request(&mut stream).unwrap();
        assert!(
            request
                .lines()
                .any(|line| line.eq_ignore_ascii_case("authorization: Bearer token")),
            "authenticated endpoint omitted its session token: {request}"
        );
        write!(
            stream,
            "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown"),
            body.len(),
        )
        .unwrap();
        request
    });
    (format!("http://{address}"), server)
}
