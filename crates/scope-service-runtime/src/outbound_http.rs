use opentelemetry::global;
use opentelemetry_http::HeaderInjector;
use reqwest::{RequestBuilder, Response};
use tracing::Instrument as _;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

pub async fn send_traced(
    request: RequestBuilder,
    route: &'static str,
) -> reqwest::Result<Response> {
    let (client, request) = request.build_split();
    let mut request = request?;
    let span = tracing::info_span!(
        target: "scope_service_runtime",
        "HTTP request",
        otel.kind = "client",
        http.request.method = %request.method(),
        server.address = request.url().host_str().unwrap_or("unknown"),
        http.route = route,
        http.response.status_code = tracing::field::Empty,
    );
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&span.context(), &mut HeaderInjector(request.headers_mut()));
    });
    let response = client.execute(request).instrument(span.clone()).await?;
    span.record("http.response.status_code", response.status().as_u16());
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::HeaderMap, routing::get};
    use opentelemetry::trace::{TraceContextExt as _, TracerProvider as _};
    use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
    use tracing_subscriber::prelude::*;

    #[tokio::test(flavor = "current_thread")]
    async fn outbound_request_carries_current_trace_context() {
        global::set_text_map_propagator(TraceContextPropagator::new());
        let provider = SdkTracerProvider::builder().build();
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer().with_tracer(provider.tracer("outbound-http-test")),
        );
        let _guard = tracing::subscriber::set_default(subscriber);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/",
                    get(|headers: HeaderMap| async move {
                        headers
                            .get("traceparent")
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or_default()
                            .to_string()
                    }),
                ),
            )
            .await
            .unwrap();
        });

        let parent = tracing::info_span!(target: "scope_service_runtime", "parent");
        let expected_trace_id = parent
            .context()
            .span()
            .span_context()
            .trace_id()
            .to_string();
        let response = async {
            send_traced(
                reqwest::Client::new().get(format!("http://{address}/")),
                "/",
            )
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
        }
        .instrument(parent)
        .await;
        let parts = response.split('-').collect::<Vec<_>>();
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], "00");
        assert_eq!(parts[1], expected_trace_id);
        assert_eq!(parts[2].len(), 16);
        assert!(parts[2].bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(parts[3], "01");
    }
}
