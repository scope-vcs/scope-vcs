//! Database-independent responses while a release owns the writer fence.
use axum::{
    Router,
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};

pub fn router() -> Router {
    Router::new()
        .route("/readyz", get(|| async { "maintenance ready" }))
        .fallback(unavailable)
}

async fn unavailable(headers: HeaderMap) -> Response {
    let browser = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|part| part.trim().starts_with("text/html"))
        });
    let mut response = if browser {
        Html("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Scope maintenance</title><style>html{color-scheme:light dark}body{font:18px/1.6 system-ui,sans-serif;margin:0;padding:clamp(24px,8vw,96px)}main{max-width:42rem;margin:12vh auto}h1{font-size:clamp(28px,5vw,44px);line-height:1.15}p{opacity:.75}</style><body><main><h1>Scope is updating</h1><p>Scheduled maintenance is in progress. Please try again shortly.</p></main></body></html>").into_response()
    } else {
        axum::Json(serde_json::json!({"error":"maintenance","message":"Scope is updating. Please try again shortly."})).into_response()
    };
    *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, "60".parse().unwrap());
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

pub async fn serve() -> anyhow::Result<()> {
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".into())
        .parse()?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).await?;
    axum::serve(listener, router())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn maintenance_handles_browser_api_git_and_media_without_state() {
        for (path, accept) in [
            ("/", "text/html"),
            ("/api/repos", "application/json"),
            ("/repo.git/git-receive-pack", "*/*"),
            ("/media/file", "*/*"),
        ] {
            let response = router()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header(header::ACCEPT, accept)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(response.headers()[header::RETRY_AFTER], "60");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            assert!(
                String::from_utf8_lossy(&body).contains(if accept == "text/html" {
                    "<h1>"
                } else {
                    "\"error\":\"maintenance\""
                })
            );
        }
    }

    #[tokio::test]
    async fn readiness_does_not_claim_application_health() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
