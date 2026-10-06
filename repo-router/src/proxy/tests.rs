use super::*;
use crate::{RouterConfig, app::test_router_with_state, discovery::BackendDiscovery};
use axum::http::Method;
use axum::{Router, routing::any};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tower::ServiceExt;

async fn identified_backend() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let identity = address.to_string();
    let upstream = Router::new().fallback(any(move || {
        let identity = identity.clone();
        async move { identity }
    }));
    tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    address
}

async fn dead_rank_one_with_live_router(
    upstream: Router,
    read_timeout: Duration,
    replay_max_bytes: usize,
) -> Router {
    let first = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_address = first.local_addr().unwrap();
    let second_address = second.local_addr().unwrap();
    let addresses = vec![first_address, second_address];
    let identities = addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let rank_one = rank_backends("scope/router", &identities, Clone::clone)[0];
    let live_listener = if *rank_one == first_address.to_string() {
        drop(first);
        second
    } else {
        drop(second);
        first
    };
    tokio::spawn(async move { axum::serve(live_listener, upstream).await.unwrap() });
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(100))
        .read_timeout(read_timeout)
        .build()
        .unwrap();
    test_router_with_state(
        BackendDiscovery::fixed(addresses),
        http,
        2,
        replay_max_bytes,
    )
}

async fn selected_backend(router: &Router, method: Method, uri: &str) -> String {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    String::from_utf8(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}

#[tokio::test]
async fn streams_git_requests_and_responses_through_the_selected_backend() {
    let upstream = Router::new().fallback(any(|request: Request| async move {
        let marker = request.headers().get("x-test-marker").cloned().unwrap();
        let mut response = Response::new(request.into_body());
        response.headers_mut().insert("x-test-marker", marker);
        response
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let router = test_router_with_state(
        BackendDiscovery::fixed(vec![address]),
        reqwest::Client::new(),
        1,
        64 * 1024 * 1024,
    );
    let request = Request::builder()
        .method("POST")
        .uri("/git/permissioned/scope/router/git-upload-pack")
        .header("x-test-marker", "preserved")
        .body(Body::from("pack request"))
        .unwrap();

    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-test-marker"], "preserved");
    assert_eq!(response.headers()[ROUTER_RESPONSE_HEADER], "1");
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap(),
        "pack request"
    );
}

#[tokio::test]
async fn upload_pack_get_and_post_fail_over_after_a_connect_failure() {
    for (method, body) in [
        (Method::GET, Body::empty()),
        (Method::POST, Body::from("replayable pack request")),
    ] {
        let is_post = method == Method::POST;
        let upstream = Router::new().fallback(any(|request: Request| async move {
            Response::new(request.into_body())
        }));
        let router = dead_rank_one_with_live_router(upstream, Duration::from_secs(1), 1024).await;
        let uri = if method == Method::GET {
            "/git/public/scope/router/info/refs?service=git-upload-pack"
        } else {
            "/git/public/scope/router/git-upload-pack"
        };

        let response = router
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        if is_post {
            assert_eq!(
                axum::body::to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap(),
                "replayable pack request"
            );
        }
    }
}

#[tokio::test]
async fn upload_pack_replay_body_is_strictly_bounded() {
    let requests = Arc::new(AtomicUsize::new(0));
    let upstream_requests = Arc::clone(&requests);
    let upstream = Router::new().fallback(any(move || {
        upstream_requests.fetch_add(1, Ordering::SeqCst);
        async { StatusCode::OK }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let router = test_router_with_state(
        BackendDiscovery::fixed(vec![address]),
        reqwest::Client::new(),
        1,
        4,
    );

    let response = router
        .oneshot(
            Request::post("/git/public/scope/router/git-upload-pack")
                .body(Body::from("12345"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn ends_an_upstream_read_that_stops_making_progress() {
    let upstream = Router::new().fallback(any(|| async { std::future::pending::<String>().await }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let router = crate::router(RouterConfig {
        backend_authority: address.to_string(),
        dns_refresh: std::time::Duration::from_secs(1),
        dns_max_stale: std::time::Duration::from_secs(30),
        connect_timeout: std::time::Duration::from_secs(1),
        read_timeout: std::time::Duration::from_millis(20),
        read_replicas: 1,
        upload_pack_replay_max_bytes: 1024,
        upload_pack_replay_slots: 4,
        incoming_body_timeout: Duration::from_secs(15),
    })
    .unwrap();

    let response = router
        .oneshot(
            Request::builder()
                .uri("/git/public/scope/router/info/refs?service=git-upload-pack")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn read_timeout_does_not_retry_an_ambiguous_upload_pack_attempt() {
    let first = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addresses = vec![first.local_addr().unwrap(), second.local_addr().unwrap()];
    let identities = addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let rank_one = rank_backends("scope/router", &identities, Clone::clone)[0];
    let fallback_requests = Arc::new(AtomicUsize::new(0));
    let fallback_counter = Arc::clone(&fallback_requests);
    let fallback = Router::new().fallback(any(move || {
        fallback_counter.fetch_add(1, Ordering::SeqCst);
        async { StatusCode::OK }
    }));
    let stalled = Router::new().fallback(any(|| async { std::future::pending::<String>().await }));
    if *rank_one == addresses[0].to_string() {
        tokio::spawn(async move { axum::serve(first, stalled).await.unwrap() });
        tokio::spawn(async move { axum::serve(second, fallback).await.unwrap() });
    } else {
        tokio::spawn(async move { axum::serve(second, stalled).await.unwrap() });
        tokio::spawn(async move { axum::serve(first, fallback).await.unwrap() });
    }
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(100))
        .read_timeout(Duration::from_millis(20))
        .build()
        .unwrap();
    let router = test_router_with_state(BackendDiscovery::fixed(addresses), http, 2, 1024);

    let response = router
        .oneshot(
            Request::get("/git/public/scope/router/info/refs?service=git-upload-pack")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(fallback_requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn refuses_non_git_paths() {
    let response = test_router_with_state(
        BackendDiscovery::fixed(Vec::new()),
        reqwest::Client::new(),
        1,
        64 * 1024 * 1024,
    )
    .oneshot(
        Request::builder()
            .uri("/v1/repos/scope/router")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn spreads_upload_pack_reads_across_the_ranked_prefix() {
    let first = identified_backend().await;
    let second = identified_backend().await;
    let third = identified_backend().await;
    let addresses = vec![first, second, third];
    let identities = addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let expected = rank_backends("scope/router", &identities, Clone::clone)
        .into_iter()
        .take(2)
        .cloned()
        .collect::<BTreeSet<_>>();
    let router = test_router_with_state(
        BackendDiscovery::fixed(addresses),
        reqwest::Client::new(),
        2,
        64 * 1024 * 1024,
    );

    let actual = [
        (
            Method::GET,
            "/git/public/scope/router/info/refs?service=git-upload-pack",
        ),
        (Method::POST, "/git/public/scope/router/git-upload-pack"),
        (
            Method::GET,
            "/git/permissioned/scope/router/info/refs?service=git-upload-pack",
        ),
        (
            Method::POST,
            "/git/permissioned/scope/router/git-upload-pack",
        ),
    ];
    let mut selected = BTreeSet::new();
    for (method, uri) in actual {
        selected.insert(selected_backend(&router, method, uri).await);
    }

    assert_eq!(selected, expected);
}

#[tokio::test]
async fn pins_receive_pack_operations_to_rendezvous_rank_one() {
    let first = identified_backend().await;
    let second = identified_backend().await;
    let third = identified_backend().await;
    let addresses = vec![first, second, third];
    let identities = addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let expected = rank_backends("scope/router", &identities, Clone::clone)[0];
    let router = test_router_with_state(
        BackendDiscovery::fixed(addresses),
        reqwest::Client::new(),
        3,
        64 * 1024 * 1024,
    );

    for (method, uri) in [
        (
            Method::GET,
            "/git/permissioned/scope/router/info/refs?service=git-receive-pack",
        ),
        (
            Method::POST,
            "/git/permissioned/scope/router/git-receive-pack",
        ),
    ] {
        assert_eq!(selected_backend(&router, method, uri).await, *expected);
    }
}

#[tokio::test]
async fn receive_pack_never_fails_over_or_replays() {
    let fallback_requests = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&fallback_requests);
    let fallback = Router::new().fallback(any(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        async { StatusCode::OK }
    }));
    let router = dead_rank_one_with_live_router(fallback, Duration::from_secs(1), 1024).await;

    for (method, uri, body) in [
        (
            Method::GET,
            "/git/permissioned/scope/router/info/refs?service=git-receive-pack",
            Body::empty(),
        ),
        (
            Method::POST,
            "/git/permissioned/scope/router/git-receive-pack",
            Body::from("write once"),
        ),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    assert_eq!(fallback_requests.load(Ordering::SeqCst), 0);
}

#[test]
fn strips_connection_scoped_headers_and_preserves_end_to_end_values() {
    let mut headers = HeaderMap::new();
    headers.insert(header::HOST, "scope.example".parse().unwrap());
    headers.append(
        header::CONNECTION,
        "keep-alive, x-remove-one".parse().unwrap(),
    );
    headers.append(header::CONNECTION, "x-remove-two".parse().unwrap());
    headers.insert("keep-alive", "timeout=5".parse().unwrap());
    headers.insert("x-remove-one", "one".parse().unwrap());
    headers.insert("x-remove-two", "two".parse().unwrap());
    headers.insert("proxy-connection", "keep-alive".parse().unwrap());
    headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
    headers.append("x-preserve", "first".parse().unwrap());
    headers.append("x-preserve", "second".parse().unwrap());

    let forwarded = forwarded_headers(&headers);

    for removed in [
        "host",
        "connection",
        "keep-alive",
        "proxy-connection",
        "x-remove-one",
        "x-remove-two",
    ] {
        assert!(!forwarded.contains_key(removed), "preserved {removed}");
    }
    assert_eq!(forwarded[header::AUTHORIZATION], "Bearer secret");
    assert_eq!(
        forwarded
            .get_all("x-preserve")
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
}
