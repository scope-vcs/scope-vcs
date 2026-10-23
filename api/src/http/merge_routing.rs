use crate::{error::ApiError, state::AppState};
use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderValue, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use scope_api_contract::routes;
use scope_service_runtime::{
    outbound_http::send_traced,
    proxy::{forwarded_headers, proxied_response},
    replicas::{Backend, BackendDiscovery, rank_backends},
};
use std::{net::SocketAddr, time::Duration};

const ROUTED_HEADER: &str = "x-scope-merge-routed";
const MAX_MERGE_BODY_BYTES: usize = 64 * 1024;
const PRIVATE_DOMAIN_ENV: &str = "RAILWAY_PRIVATE_DOMAIN";
const DISCOVERY_REFRESH: Duration = Duration::from_secs(1);
const DISCOVERY_MAX_STALE: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct MergeRouting {
    replicas: BackendDiscovery,
    http: reqwest::Client,
}

impl MergeRouting {
    pub(crate) fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(domain) = crate::config::non_empty_env(PRIVATE_DOMAIN_ENV) else {
            return Ok(None);
        };
        let port = scope_service_runtime::port_from_env(8080);
        Ok(Some(Self::new(BackendDiscovery::new(
            &format!("{domain}:{port}"),
            DISCOVERY_REFRESH,
            DISCOVERY_MAX_STALE,
        ))?))
    }

    #[cfg(test)]
    pub(crate) fn fixed(replicas: Vec<SocketAddr>) -> Self {
        Self::new(BackendDiscovery::fixed(replicas)).unwrap()
    }

    fn new(replicas: BackendDiscovery) -> anyhow::Result<Self> {
        Ok(Self {
            replicas,
            http: reqwest::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .build()?,
        })
    }

    async fn owner(&self, repository: &str) -> Option<SocketAddr> {
        let discovered = match self.replicas.backends().await {
            Ok(discovered) => discovered,
            Err(error) => {
                tracing::warn!(repository, %error, "merging locally without replica discovery");
                return None;
            }
        };
        rank_backends(repository, &discovered.backends, |backend: &Backend| {
            backend.address.to_string()
        })
        .first()
        .map(|backend| backend.address)
    }
}

pub(crate) async fn route_to_owner(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(routing) = state.merge_routing.as_deref() else {
        return next.run(request).await;
    };
    if request.headers().contains_key(ROUTED_HEADER) {
        return next.run(request).await;
    }
    let Some(repository) = repository_key(request.uri().path()) else {
        return next.run(request).await;
    };
    let (parts, body) = request.into_parts();
    let body = match axum::body::to_bytes(body, MAX_MERGE_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => return ApiError::payload_too_large(error.to_string()).into_response(),
    };
    let Some(owner) = routing.owner(&repository).await else {
        return next.run(Request::from_parts(parts, Body::from(body))).await;
    };
    match forward(routing, owner, &parts, body.clone()).await {
        Ok(response) => proxied_response(response),
        Err(error) if error.is_connect() => {
            tracing::warn!(%repository, %owner, %error, "merge owner unreachable; merging locally");
            next.run(Request::from_parts(parts, Body::from(body))).await
        }
        Err(error) => ApiError::upstream_unavailable(
            "The merge did not finish; check the request before retrying.",
            format!("merge forwarded to {owner} for {repository} failed: {error}"),
        )
        .into_response(),
    }
}

async fn forward(
    routing: &MergeRouting,
    owner: SocketAddr,
    parts: &Parts,
    body: Bytes,
) -> reqwest::Result<reqwest::Response> {
    let path = parts
        .uri
        .path_and_query()
        .map_or_else(|| parts.uri.path(), |path| path.as_str());
    let mut headers = forwarded_headers(&parts.headers);
    headers.insert(ROUTED_HEADER, HeaderValue::from_static("1"));
    send_traced(
        routing
            .http
            .request(parts.method.clone(), format!("http://{owner}{path}"))
            .headers(headers)
            .body(body),
        routes::REPO_REQUEST_MERGE,
    )
    .await
}

fn repository_key(path: &str) -> Option<String> {
    let mut segments = path.strip_prefix("/v1/repos/")?.split('/');
    let owner = segments.next().filter(|owner| !owner.is_empty())?;
    let repository = segments.next().filter(|name| !name.is_empty())?;
    Some(format!("{owner}/{repository}"))
}
