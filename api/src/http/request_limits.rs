use crate::error::ApiError;
use axum::{
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, HeaderValue, header::RETRY_AFTER},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::MethodRouter,
};
use sha2::{Digest as _, Sha256};
use std::{
    collections::HashMap,
    hash::Hash,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::time::Instant;

pub(crate) const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
pub(crate) const DEFAULT_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;

const PROXY_CLIENT_ADDRESS_HEADER: &str = "x-real-ip";
const FIRST_PRUNE_AT_TRACKED_KEYS: usize = 10_000;

pub(crate) async fn enforce_default_timeout(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    match tokio::time::timeout(DEFAULT_REQUEST_TIMEOUT, next.run(request)).await {
        Ok(response) => response,
        Err(_) => ApiError::infrastructure_unavailable(format!(
            "{method} {path} exceeded the {}s request deadline",
            DEFAULT_REQUEST_TIMEOUT.as_secs()
        ))
        .into_response(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ClientAddress {
    V4([u8; 4]),
    V6Network([u8; 8]),
}

impl ClientAddress {
    fn of(headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<Self> {
        headers
            .get(PROXY_CLIENT_ADDRESS_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
            .or(peer.map(|peer| peer.ip()))
            .map(Self::from)
    }
}

impl From<IpAddr> for ClientAddress {
    fn from(address: IpAddr) -> Self {
        match address.to_canonical() {
            IpAddr::V4(address) => Self::V4(address.octets()),
            IpAddr::V6(address) => {
                let mut network = [0; 8];
                network.copy_from_slice(&address.octets()[..8]);
                Self::V6Network(network)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LoginCode([u8; 16]);

impl LoginCode {
    fn of(code: &str) -> Self {
        let mut digest = [0; 16];
        digest.copy_from_slice(&Sha256::digest(code.as_bytes())[..16]);
        Self(digest)
    }
}

struct RateLimit<K> {
    burst: u32,
    refill_interval: Duration,
    spending: Mutex<Spending<K>>,
}

struct Spending<K> {
    replenished_at: HashMap<K, Instant>,
    prune_at_tracked_keys: usize,
}

impl<K> Default for Spending<K> {
    fn default() -> Self {
        Self {
            replenished_at: HashMap::new(),
            prune_at_tracked_keys: FIRST_PRUNE_AT_TRACKED_KEYS,
        }
    }
}

pub(crate) fn login_start_limit<S>() -> impl Fn(MethodRouter<S>) -> MethodRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    let per_address = Arc::new(RateLimit::<ClientAddress>::new(10, Duration::from_secs(6)));
    move |route| {
        route.route_layer(middleware::from_fn_with_state(
            per_address.clone(),
            limit_per_address,
        ))
    }
}

pub(crate) fn login_exchange_limit<S>() -> impl Fn(MethodRouter<S>) -> MethodRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    let per_address = Arc::new(RateLimit::<ClientAddress>::new(60, Duration::from_secs(1)));
    move |route| {
        route.route_layer(middleware::from_fn_with_state(
            per_address.clone(),
            limit_per_address,
        ))
    }
}

pub(crate) fn login_poll_limit<S>() -> impl Fn(MethodRouter<S>) -> MethodRouter<S>
where
    S: Clone + Send + Sync + 'static,
{
    let per_code = Arc::new(RateLimit::<LoginCode>::new(10, Duration::from_secs(1)));
    let per_address = Arc::new(RateLimit::<ClientAddress>::new(
        300,
        Duration::from_millis(100),
    ));
    move |route| {
        route
            .route_layer(middleware::from_fn_with_state(
                per_code.clone(),
                limit_per_login_code,
            ))
            .route_layer(middleware::from_fn_with_state(
                per_address.clone(),
                limit_per_address,
            ))
    }
}

impl<K: Copy + Eq + Hash> RateLimit<K> {
    fn new(burst: u32, refill_interval: Duration) -> Self {
        Self {
            burst,
            refill_interval,
            spending: Mutex::default(),
        }
    }

    fn admit(&self, key: K, now: Instant) -> Result<(), Duration> {
        let mut spending = self.spending.lock().unwrap_or_else(PoisonError::into_inner);
        let Spending {
            replenished_at,
            prune_at_tracked_keys,
        } = &mut *spending;
        if replenished_at.len() >= *prune_at_tracked_keys {
            replenished_at.retain(|_, at| *at > now);
            *prune_at_tracked_keys = FIRST_PRUNE_AT_TRACKED_KEYS.max(replenished_at.len() * 2);
        }
        let spent_until = replenished_at
            .get(&key)
            .copied()
            .filter(|at| *at > now)
            .unwrap_or(now);
        let burst_window = self.refill_interval * (self.burst - 1);
        let retry_after = spent_until.saturating_duration_since(now + burst_window);
        if !retry_after.is_zero() {
            return Err(retry_after);
        }
        replenished_at.insert(key, spent_until + self.refill_interval);
        Ok(())
    }
}

async fn limit_per_address(
    State(limit): State<Arc<RateLimit<ClientAddress>>>,
    request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| *peer);
    let admitted = ClientAddress::of(request.headers(), peer)
        .map_or(Ok(()), |address| limit.admit(address, Instant::now()));
    match admitted {
        Ok(()) => next.run(request).await,
        Err(retry_after) => throttled(
            "Too many requests from this address; retry shortly",
            retry_after,
        ),
    }
}

async fn limit_per_login_code(
    State(limit): State<Arc<RateLimit<LoginCode>>>,
    Path(code): Path<String>,
    request: Request,
    next: Next,
) -> Response {
    match limit.admit(LoginCode::of(&code), Instant::now()) {
        Ok(()) => next.run(request).await,
        Err(retry_after) => throttled(
            "This login is polling too often; retry shortly",
            retry_after,
        ),
    }
}

fn throttled(message: &str, retry_after: Duration) -> Response {
    let mut response = ApiError::too_many_requests(message).into_response();
    let seconds = retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0);
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from(seconds));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::StatusCode,
        routing::{get, post},
    };
    use tower::ServiceExt as _;

    fn limit(burst: u32) -> RateLimit<ClientAddress> {
        RateLimit::new(burst, Duration::from_secs(1))
    }

    fn address(text: &str) -> ClientAddress {
        text.parse::<IpAddr>().unwrap().into()
    }

    #[test]
    fn an_address_spends_its_burst_then_regains_one_request_per_interval() {
        let limit = limit(3);
        let start = Instant::now();
        let client = address("203.0.113.7");
        for _ in 0..3 {
            limit.admit(client, start).unwrap();
        }
        assert_eq!(limit.admit(client, start), Err(Duration::from_secs(1)));
        limit.admit(address("203.0.113.8"), start).unwrap();
        let later = start + Duration::from_millis(1_500);
        limit.admit(client, later).unwrap();
        assert_eq!(limit.admit(client, later), Err(Duration::from_millis(500)));
        let idle = start + Duration::from_secs(60);
        for _ in 0..3 {
            limit.admit(client, idle).unwrap();
        }
    }

    #[test]
    fn replenished_addresses_are_forgotten_once_many_are_tracked() {
        let limit = limit(1);
        let start = Instant::now();
        for host in 0..FIRST_PRUNE_AT_TRACKED_KEYS {
            let octets = u32::try_from(host).unwrap().to_be_bytes();
            limit.admit(ClientAddress::V4(octets), start).unwrap();
        }
        limit
            .admit(address("2001:db8::1"), start + Duration::from_secs(2))
            .unwrap();
        assert_eq!(limit.spending.lock().unwrap().replenished_at.len(), 1);
    }

    #[test]
    fn the_proxy_header_names_the_client_and_other_forwarding_headers_do_not() {
        let peer = Some("198.51.100.4:4000".parse().unwrap());
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.9"));
        headers.insert("forwarded", HeaderValue::from_static("for=203.0.113.9"));
        assert_eq!(
            ClientAddress::of(&headers, peer),
            Some(address("198.51.100.4"))
        );
        assert_eq!(ClientAddress::of(&headers, None), None);
        headers.insert("x-real-ip", HeaderValue::from_static("203.0.113.7"));
        assert_eq!(
            ClientAddress::of(&headers, peer),
            Some(address("203.0.113.7"))
        );
        assert_eq!(
            address("2001:db8:1:2:aaaa::1"),
            address("2001:db8:1:2:bbbb::2")
        );
        assert_ne!(address("2001:db8:1:2::1"), address("2001:db8:1:3::1"));
        assert_eq!(address("::ffff:203.0.113.7"), address("203.0.113.7"));
    }

    #[tokio::test]
    async fn a_limited_route_answers_429_with_retry_after_for_the_spent_address_only() {
        let app = Router::new()
            .route("/", get(|| async { StatusCode::NO_CONTENT }))
            .route_layer(middleware::from_fn_with_state(
                Arc::new(limit(1)),
                limit_per_address,
            ));
        let call = |client: &'static str| {
            let request = Request::builder()
                .uri("/")
                .header("x-real-ip", client)
                .body(Body::empty())
                .unwrap();
            app.clone().oneshot(request)
        };
        assert_eq!(
            call("203.0.113.7").await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
        let limited = call("203.0.113.7").await.unwrap();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(limited.headers()[RETRY_AFTER], "1");
        assert_eq!(
            call("203.0.113.8").await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_past_the_default_deadline_is_answered_as_unavailable() {
        let app = Router::new()
            .route("/", get(std::future::pending::<StatusCode>))
            .layer(middleware::from_fn(enforce_default_timeout));
        let started = Instant::now();
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(started.elapsed(), DEFAULT_REQUEST_TIMEOUT);
    }

    #[tokio::test(start_paused = true)]
    async fn several_logins_behind_one_address_poll_freely_while_a_hammered_code_is_throttled() {
        let app = Router::new().route(
            "/poll/{code}",
            login_poll_limit()(post(|| async { StatusCode::NO_CONTENT })),
        );
        let poll = |code: usize| {
            let request = Request::builder()
                .method("POST")
                .uri(format!("/poll/code-{code}"))
                .header("x-real-ip", "203.0.113.7")
                .body(Body::empty())
                .unwrap();
            app.clone().oneshot(request)
        };
        for _round in 0..300 {
            for login in 0..12 {
                assert_eq!(poll(login).await.unwrap().status(), StatusCode::NO_CONTENT);
            }
            tokio::time::advance(Duration::from_secs(2)).await;
        }

        let mut hammered = Vec::new();
        for _ in 0..11 {
            hammered.push(poll(0).await.unwrap());
        }
        let refused = hammered.pop().unwrap();
        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(refused.headers()[RETRY_AFTER], "1");
        assert!(
            hammered
                .iter()
                .all(|admitted| admitted.status() == StatusCode::NO_CONTENT)
        );
        assert_eq!(poll(1).await.unwrap().status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn one_address_inventing_login_codes_is_throttled_by_address() {
        let app = Router::new().route(
            "/poll/{code}",
            login_poll_limit()(post(|| async { StatusCode::NO_CONTENT })),
        );
        let poll = |address: &'static str, code: usize| {
            let request = Request::builder()
                .method("POST")
                .uri(format!("/poll/invented-{code}"))
                .header("x-real-ip", address)
                .body(Body::empty())
                .unwrap();
            app.clone().oneshot(request)
        };
        let mut admitted = 0;
        while poll("203.0.113.7", admitted).await.unwrap().status() == StatusCode::NO_CONTENT {
            admitted += 1;
            assert!(admitted < 10_000, "invented codes were never throttled");
        }
        assert!(admitted >= 300);
        assert_eq!(
            poll("203.0.113.8", admitted).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
    }
}
