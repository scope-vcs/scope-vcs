use http_body::{Body as HttpBody, Frame, SizeHint};
use opentelemetry::global;
use opentelemetry_http::HeaderInjector;
use reqwest::{Body, RequestBuilder, Response, ResponseBuilderExt as _};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
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
        otel.status_code = tracing::field::Empty,
        http.request.method = %request.method(),
        server.address = request.url().host_str().unwrap_or("unknown"),
        http.route = route,
        http.response.status_code = tracing::field::Empty,
    );
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&span.context(), &mut HeaderInjector(request.headers_mut()));
    });
    let response = match client.execute(request).instrument(span.clone()).await {
        Ok(response) => response,
        Err(error) => {
            span.record("otel.status_code", "ERROR");
            return Err(error);
        }
    };
    span.record("http.response.status_code", response.status().as_u16());
    if response.status().is_server_error() {
        span.record("otel.status_code", "ERROR");
    }
    Ok(with_span_until_body_ends(response, span))
}

fn with_span_until_body_ends(response: Response, span: tracing::Span) -> Response {
    let url = response.url().clone();
    let (mut parts, body) = http::Response::<Body>::from(response).into_parts();
    let (url_parts, ()) = http::Response::builder()
        .url(url)
        .body(())
        .expect("an empty response with a URL is valid")
        .into_parts();
    parts.extensions.extend(url_parts.extensions);
    Response::from(http::Response::from_parts(
        parts,
        Body::wrap(SpannedBody { inner: body, span }),
    ))
}

struct SpannedBody {
    inner: Body,
    span: tracing::Span,
}

impl HttpBody for SpannedBody {
    type Data = bytes::Bytes;
    type Error = reqwest::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let span = self.span.clone();
        let _entered = span.enter();
        let frame = Pin::new(&mut self.inner).poll_frame(context);
        if let Poll::Ready(Some(Err(_))) = &frame {
            span.record("otel.status_code", "ERROR");
        }
        frame
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::HeaderMap, routing::get};
    use opentelemetry::trace::{Status, TraceContextExt as _, TracerProvider as _};
    use opentelemetry_sdk::{
        error::OTelSdkResult,
        propagation::TraceContextPropagator,
        trace::{SdkTracerProvider, SpanData, SpanExporter},
    };
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
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

    #[derive(Clone, Debug, Default)]
    struct Exported(Arc<Mutex<Vec<SpanData>>>);

    impl SpanExporter for Exported {
        async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
            self.0.lock().unwrap().extend(batch);
            Ok(())
        }
    }

    async fn serve_once(status_line: &'static str, body_delay: Duration) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(format!("{status_line}\r\ncontent-length: 2\r\n\r\n").as_bytes())
                .await
                .unwrap();
            stream.flush().await.unwrap();
            tokio::time::sleep(body_delay).await;
            stream.write_all(b"ok").await.unwrap();
        });
        address
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spans_mark_failures_and_last_until_the_body_is_read() {
        let exported = Exported::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exported.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(
            tracing_opentelemetry::layer().with_tracer(provider.tracer("outbound-http-test")),
        );
        let _guard = tracing::subscriber::set_default(subscriber);
        let client = reqwest::Client::new();

        let failing = serve_once("HTTP/1.1 500 Internal Server Error", Duration::ZERO).await;
        drop(
            send_traced(client.get(format!("http://{failing}/")), "/")
                .await
                .unwrap(),
        );
        let refused = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(
            send_traced(client.get(format!("http://{refused}/")), "/")
                .await
                .is_err()
        );
        let slow = serve_once("HTTP/1.1 200 OK", Duration::from_millis(200)).await;
        let response = send_traced(client.get(format!("http://{slow}/")), "/")
            .await
            .unwrap();
        assert_eq!(response.bytes().await.unwrap(), "ok");

        let spans = exported.0.lock().unwrap().clone();
        assert_eq!(spans.len(), 3);
        assert!(matches!(spans[0].status, Status::Error { .. }));
        assert!(matches!(spans[1].status, Status::Error { .. }));
        assert_eq!(spans[2].status, Status::Unset);
        let elapsed = spans[2]
            .end_time
            .duration_since(spans[2].start_time)
            .unwrap();
        assert!(elapsed >= Duration::from_millis(200), "{elapsed:?}");
    }
}
