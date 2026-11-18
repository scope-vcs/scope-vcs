use super::*;
use axum::{
    body::Body,
    http::{
        Request, StatusCode,
        header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_HEADERS,
            ACCESS_CONTROL_REQUEST_METHOD, ORIGIN,
        },
    },
};
use scope_storage::{ObjectStore, ObjectStoreError};
use tower::ServiceExt;

#[tokio::test]
async fn health_and_readiness_cover_live_and_unavailable_dependencies() {
    let app = crate::router(AppState::test_state());
    let health = app
        .clone()
        .oneshot(request("GET", "/healthz"))
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(
        response_json(health).await,
        serde_json::json!({"status": "ok", "service": "api"})
    );

    let ready = app.oneshot(request("GET", "/readyz")).await.unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    assert_eq!(
        response_json(ready).await,
        serde_json::json!({
            "status": "ok",
            "service": "api",
            "checks": [
                {"name": "database", "status": "ok"},
                {"name": "object_store", "status": "ok"}
            ]
        })
    );

    let mut state = AppState::test_state();
    state.object_store = Arc::new(UnavailableObjectStore);
    let unavailable = crate::router(state)
        .oneshot(request("GET", "/readyz"))
        .await
        .unwrap();
    assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = response_json(unavailable).await;
    assert_eq!(json["checks"][1]["status"], "unavailable");
    assert!(!json.to_string().contains("secret"));
    assert!(!json.to_string().contains("internal"));
}

#[tokio::test]
async fn cors_preflight_explicitly_allows_authorization_get_and_put() {
    let response = crate::router(AppState::test_state())
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/v1/repos/owner/repo/events")
                .header(ORIGIN, "http://localhost:5173")
                .header(ACCESS_CONTROL_REQUEST_METHOD, "GET")
                .header(ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    let allow_headers = response.headers()[ACCESS_CONTROL_ALLOW_HEADERS]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert_ne!(allow_headers, "*");
    assert!(allow_headers.contains("authorization"));
    let allow_methods = response.headers()[ACCESS_CONTROL_ALLOW_METHODS]
        .to_str()
        .unwrap();
    assert_ne!(allow_methods, "*");
    assert!(allow_methods.contains("GET"));
    assert!(allow_methods.contains("PUT"));
}

fn request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

struct UnavailableObjectStore;

#[async_trait::async_trait]
impl ObjectStore for UnavailableObjectStore {
    async fn put(&self, _key: &str, _bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
        Ok(())
    }

    async fn read_to(
        &self,
        _key: &str,
        _max_bytes: u64,
        _output: &mut (dyn tokio::io::AsyncWrite + Send + Unpin),
    ) -> Result<u64, ObjectStoreError> {
        Ok(0)
    }

    async fn delete(&self, _key: &str) -> Result<(), ObjectStoreError> {
        Ok(())
    }

    async fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        Err(ObjectStoreError::service_unavailable(
            "secret internal object-store hostname is unavailable",
        ))
    }
}

#[tokio::test]
async fn bodies_over_the_default_limit_are_refused_and_git_routes_keep_their_own_limit() {
    let app = crate::router(test_state_with_readme().await);
    let oversized = crate::http::request_limits::DEFAULT_REQUEST_BODY_BYTES + 1;
    let post = |uri: &'static str, declares_length: bool| {
        let mut request = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        let body = if declares_length {
            request = request.header("content-length", oversized);
            Body::from(vec![b' '; oversized])
        } else {
            Body::from_stream(futures_util::stream::iter([oversized - 1, 1].map(
                |bytes| Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![b' '; bytes])),
            )))
        };
        app.clone().oneshot(request.body(body).unwrap())
    };
    for declares_length in [true, false] {
        assert_eq!(
            post("/v1/repos", declares_length).await.unwrap().status(),
            StatusCode::PAYLOAD_TOO_LARGE,
            "declares length: {declares_length}"
        );
        for git in [
            "/git/public/owner/repo/git-upload-pack",
            "/git/public/owner/repo/git-receive-pack",
        ] {
            assert_ne!(
                post(git, declares_length).await.unwrap().status(),
                StatusCode::PAYLOAD_TOO_LARGE,
                "{git}, declares length: {declares_length}"
            );
        }
    }
}
