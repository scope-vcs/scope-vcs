use crate::{
    app::RouterState,
    backend_selection::GitRequestKind,
    discovery::{Backend, DiscoveryFreshness},
    rendezvous::rank_backends,
    repository_path::repository_key,
};
use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderName, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use scope_service_runtime::outbound_http::send_traced;
use std::{collections::HashSet, sync::Arc};

const ROUTER_RESPONSE_HEADER: &str = "x-scope-git-router";

struct UpstreamRequest {
    method: Method,
    path_and_query: String,
    headers: HeaderMap,
}

struct RouteContext<'a> {
    repository: &'a str,
    ranked: &'a [&'a Backend],
    freshness: DiscoveryFreshness,
}

impl RouteContext<'_> {
    fn backend(&self, rank: usize) -> &Backend {
        self.ranked[rank]
    }
}

pub(crate) async fn repository_request(
    State(state): State<Arc<RouterState>>,
    request: Request,
) -> Response {
    let Some(repository) = repository_key(request.uri().path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let discovery = match state.discovery.backends().await {
        Ok(discovery) => discovery,
        Err(error) => return upstream_unavailable(&repository, error),
    };
    let kind = GitRequestKind::classify(request.method(), request.uri());
    let ranked = rank_backends(&repository, &discovery.backends, |backend| {
        backend.address.to_string()
    });
    let candidate_ranks = state.selector.candidate_indices(kind, ranked.len());
    if candidate_ranks.is_empty() {
        return upstream_unavailable(&repository, "no API replicas are available");
    }

    let upstream_request = UpstreamRequest {
        method: request.method().clone(),
        path_and_query: request
            .uri()
            .path_and_query()
            .map(|value| value.as_str())
            .unwrap_or_else(|| request.uri().path())
            .to_string(),
        headers: forwarded_headers(request.headers()),
    };
    let route = RouteContext {
        repository: &repository,
        ranked: &ranked,
        freshness: discovery.freshness,
    };

    match kind {
        GitRequestKind::UploadPackRead => {
            let (_permit, body) = match state.replay.collect(request.into_body()).await {
                Ok(buffered) => buffered,
                Err(error) => return error.into_response(),
            };
            forward_upload_pack(&state, &route, &candidate_ranks, upstream_request, body).await
        }
        GitRequestKind::PrimaryOnly => {
            let rank = candidate_ranks[0];
            let backend = route.backend(rank);
            route_telemetry(&route, kind, backend, rank, 1, 1, &upstream_request.method);
            let url = upstream_url(backend, &upstream_request.path_and_query);
            let route_template = upstream_route(&upstream_request.path_and_query);
            match send_traced(
                state
                    .http
                    .request(upstream_request.method, url)
                    .headers(upstream_request.headers)
                    .body(reqwest::Body::wrap_stream(
                        request.into_body().into_data_stream(),
                    )),
                route_template,
            )
            .await
            {
                Ok(response) => upstream_response(response),
                Err(error) => upstream_unavailable(route.repository, error),
            }
        }
    }
}

async fn forward_upload_pack(
    state: &RouterState,
    route: &RouteContext<'_>,
    candidate_ranks: &[usize],
    request: UpstreamRequest,
    body: Bytes,
) -> Response {
    for (attempt_index, &rank) in candidate_ranks.iter().enumerate() {
        let backend = route.backend(rank);
        route_telemetry(
            route,
            GitRequestKind::UploadPackRead,
            backend,
            rank,
            attempt_index + 1,
            candidate_ranks.len(),
            &request.method,
        );
        let response = send_traced(
            state
                .http
                .request(
                    request.method.clone(),
                    upstream_url(backend, &request.path_and_query),
                )
                .headers(request.headers.clone())
                .body(body.clone()),
            upstream_route(&request.path_and_query),
        )
        .await;
        match response {
            Ok(response) => {
                if attempt_index > 0 {
                    tracing::info!(
                        repository = route.repository,
                        backend = %backend.address,
                        backend_rank = rank + 1,
                        attempt = attempt_index + 1,
                        discovery_state = ?route.freshness,
                        "Git upload-pack failover succeeded"
                    );
                }
                return upstream_response(response);
            }
            Err(error) if error.is_connect() && attempt_index + 1 < candidate_ranks.len() => {
                tracing::warn!(
                    repository = route.repository,
                    backend = %backend.address,
                    backend_rank = rank + 1,
                    attempt = attempt_index + 1,
                    candidate_count = candidate_ranks.len(),
                    %error,
                    "Git upload-pack backend connection failed; trying next ranked replica"
                );
            }
            Err(error) => return upstream_unavailable(route.repository, error),
        }
    }

    upstream_unavailable(route.repository, "no API replicas are available")
}

fn upstream_url(backend: &Backend, path_and_query: &str) -> String {
    format!("http://{}{path_and_query}", backend.address)
}

fn upstream_route(path_and_query: &str) -> &'static str {
    let path = path_and_query.split('?').next().unwrap_or_default();
    if path.ends_with("/info/refs") {
        "/git/{mode}/{owner}/{repo}/info/refs"
    } else if path.ends_with("/git-upload-pack") {
        "/git/{mode}/{owner}/{repo}/git-upload-pack"
    } else if path.ends_with("/git-receive-pack") {
        "/git/{mode}/{owner}/{repo}/git-receive-pack"
    } else {
        "/git/{mode}/{owner}/{repo}/{operation}"
    }
}

fn route_telemetry(
    route: &RouteContext<'_>,
    kind: GitRequestKind,
    backend: &Backend,
    rank: usize,
    attempt: usize,
    candidate_count: usize,
    method: &Method,
) {
    tracing::info!(
        repository = route.repository,
        backend = %backend.address,
        backend_rank = rank + 1,
        attempt,
        candidate_count,
        failover = attempt > 1,
        discovery_state = ?route.freshness,
        ?kind,
        %method,
        "routing Git request"
    );
}

fn upstream_response(upstream: reqwest::Response) -> Response {
    let status = upstream.status();
    let headers = forwarded_headers(upstream.headers());
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response.headers_mut().insert(
        ROUTER_RESPONSE_HEADER,
        "1".parse().expect("static header value"),
    );
    response
}

fn forwarded_headers(headers: &HeaderMap) -> HeaderMap {
    let connection_headers = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| {
            value
                .split(',')
                .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        })
        .collect::<HashSet<_>>();
    headers
        .iter()
        .filter(|(name, _)| !is_hop_by_hop(name) && !connection_headers.contains(*name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
    )
}

fn upstream_unavailable(repository: &str, error: impl std::fmt::Display) -> Response {
    tracing::warn!(repository, %error, "Git router upstream unavailable");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Git service is temporarily unavailable",
    )
        .into_response()
}

#[cfg(test)]
mod tests;
