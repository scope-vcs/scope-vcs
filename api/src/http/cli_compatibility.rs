use crate::{auth::clerk::bearer_token, config::CLI_SESSION_TOKEN_PREFIX};
use axum::{
    Json,
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use scope_api_contract::{CLI_PROTOCOL_HEADER, CLI_PROTOCOL_VERSION, ErrorResponse};

pub(crate) async fn enforce(request: Request, next: Next) -> Response {
    if let Some(installed_protocol) = incompatible_cli_protocol(&request) {
        return (
            axum::http::StatusCode::UPGRADE_REQUIRED,
            Json(ErrorResponse::cli_upgrade_required(installed_protocol)),
        )
            .into_response();
    }

    next.run(request).await
}

fn incompatible_cli_protocol(request: &Request) -> Option<Option<u32>> {
    let path = request.uri().path();
    if !path.starts_with("/v1/") || path.starts_with("/v1/cli/") {
        return None;
    }
    let is_cli_session = bearer_token(request.headers())
        .ok()
        .flatten()
        .is_some_and(|token| token.starts_with(CLI_SESSION_TOKEN_PREFIX));
    if !is_cli_session {
        return None;
    }

    let installed = request
        .headers()
        .get(CLI_PROTOCOL_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok());
    (installed != Some(CLI_PROTOCOL_VERSION)).then_some(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Method, Request as HttpRequest, StatusCode, header::AUTHORIZATION},
        middleware,
        routing::{get, post},
    };
    use scope_api_contract::ErrorCode;
    use tower::ServiceExt;

    fn test_router() -> Router {
        Router::new()
            .route("/v1/mutate", post(|| async { StatusCode::NO_CONTENT }))
            .route("/v1/read", get(|| async { StatusCode::NO_CONTENT }))
            .route(
                "/v1/cli/device-login/code/complete",
                post(|| async { StatusCode::NO_CONTENT }),
            )
            .route(
                "/git/private/repo/git-receive-pack",
                post(|| async { StatusCode::NO_CONTENT }),
            )
            .layer(middleware::from_fn(enforce))
    }

    fn request(method: Method, path: &str, protocol: Option<&str>) -> HttpRequest<Body> {
        let mut request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .header(AUTHORIZATION, "Bearer scope_cli_fixture");
        if let Some(protocol) = protocol {
            request = request.header(CLI_PROTOCOL_HEADER, protocol);
        }
        request.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn incompatible_cli_reads_and_mutations_return_actionable_426() {
        let older = (CLI_PROTOCOL_VERSION - 1).to_string();
        let newer = (CLI_PROTOCOL_VERSION + 1).to_string();
        for (method, path) in [(Method::GET, "/v1/read"), (Method::POST, "/v1/mutate")] {
            for (protocol, installed) in [
                (Some(older.as_str()), Some(CLI_PROTOCOL_VERSION - 1)),
                (None, None),
                (Some(newer.as_str()), Some(CLI_PROTOCOL_VERSION + 1)),
            ] {
                let response = test_router()
                    .oneshot(request(method.clone(), path, protocol))
                    .await
                    .unwrap();

                assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
                let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
                let error: ErrorResponse = serde_json::from_slice(&body).unwrap();
                assert_eq!(error.code, ErrorCode::CliUpgradeRequired);
                assert_eq!(error.fields.installed_protocol, installed);
                assert_eq!(error.fields.supported_protocol, Some(CLI_PROTOCOL_VERSION));
            }
        }
    }

    #[tokio::test]
    async fn current_cli_and_cli_setup_or_git_traffic_pass_through() {
        let current = CLI_PROTOCOL_VERSION.to_string();
        let non_cli_bearer_mutation = HttpRequest::builder()
            .method(Method::POST)
            .uri("/v1/mutate")
            .header(AUTHORIZATION, "Bearer clerk_fixture")
            .body(Body::empty())
            .unwrap();
        for request in [
            request(Method::POST, "/v1/mutate", Some(&current)),
            request(Method::GET, "/v1/read", Some(&current)),
            request(Method::POST, "/v1/cli/device-login/code/complete", None),
            request(Method::POST, "/git/private/repo/git-receive-pack", None),
            non_cli_bearer_mutation,
        ] {
            let response = test_router().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
        }
    }
}
