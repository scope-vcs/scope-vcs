use axum_tracing_opentelemetry::middleware::OtelAxumLayer;
use opentelemetry::trace::Status;
use opentelemetry::{
    KeyValue, global,
    trace::{TraceContextExt as _, TracerProvider as _},
};
use opentelemetry_sdk::{
    Resource,
    error::OTelSdkResult,
    propagation::TraceContextPropagator,
    trace::{SdkTracerProvider, SpanData, SpanEvents, SpanExporter},
};
use std::{fmt, time::Duration};
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    EnvFilter, Layer as _,
    fmt::{FmtContext, FormatEvent, FormatFields, MakeWriter, format::Writer},
    layer::SubscriberExt as _,
    registry::LookupSpan,
    util::SubscriberInitExt as _,
};

const EXPORTED_SPANS: &str = "off,otel::tracing=trace,api=info,worker=info,scope_=info";
const UNTRACED_PATHS: [&str; 2] = ["/healthz", "/readyz"];
const EXPORTED_ATTRIBUTES: [&str; 7] = [
    "http.request.method",
    "http.response.status_code",
    "http.route",
    "network.protocol.version",
    "server.address",
    "server.port",
    "url.scheme",
];

#[must_use = "dropping telemetry without shutdown loses the final batch of spans"]
pub struct Telemetry {
    provider: Option<SdkTracerProvider>,
}

impl Telemetry {
    pub fn shutdown(self) {
        if let Some(provider) = self.provider
            && let Err(error) = provider.shutdown()
        {
            tracing::warn!(%error, "flushing exported spans failed");
        }
    }
}

pub fn init_telemetry(log_filter: &str) -> anyhow::Result<Telemetry> {
    let logs = EnvFilter::try_from_default_env().unwrap_or_else(|_| log_filter.into());
    let provider = std::env::var_os("OTEL_EXPORTER_OTLP_ENDPOINT")
        .map(|_| tracer_provider())
        .transpose()?;
    subscriber(logs, provider.as_ref(), std::io::stdout).try_init()?;
    Ok(Telemetry { provider })
}

fn subscriber<W>(
    logs: EnvFilter,
    provider: Option<&SdkTracerProvider>,
    writer: W,
) -> impl Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .event_format(WithTraceId(tracing_subscriber::fmt::format()))
                .with_writer(writer)
                .with_filter(logs),
        )
        .with(provider.map(|provider| {
            tracing_opentelemetry::layer()
                .with_tracer(provider.tracer("scope"))
                .with_filter(EnvFilter::new(EXPORTED_SPANS))
        }))
}

pub fn request_tracing() -> OtelAxumLayer {
    OtelAxumLayer::default().filter(|path| !UNTRACED_PATHS.contains(&path))
}

fn tracer_provider() -> anyhow::Result<SdkTracerProvider> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()?;
    let mut resource = Resource::builder();
    if let Ok(version) = std::env::var("OTEL_SERVICE_VERSION") {
        resource = resource.with_attribute(KeyValue::new("service.version", version));
    }
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(AllowedAttributes(exporter))
        .with_resource(resource.build())
        .build();
    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(provider.clone());
    Ok(provider)
}

#[derive(Debug)]
struct AllowedAttributes<E>(E);

impl<E: SpanExporter> SpanExporter for AllowedAttributes<E> {
    fn export(
        &self,
        mut batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        batch.iter_mut().for_each(keep_allowed_attributes);
        self.0.export(batch)
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
}

fn keep_allowed_attributes(span: &mut SpanData) {
    span.attributes
        .retain(|attribute| EXPORTED_ATTRIBUTES.contains(&attribute.key.as_str()));
    span.events = SpanEvents::default();
    if matches!(span.status, Status::Error { .. }) {
        span.status = Status::error("");
    }
}

struct WithTraceId<F>(F);

impl<S, N, F> FormatEvent<S, N> for WithTraceId<F>
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
    F: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        if let Some(trace_id) = current_trace_id() {
            write!(writer, "trace_id={trace_id} ")?;
        }
        self.0.format_event(context, writer, event)
    }
}

fn current_trace_id() -> Option<String> {
    let context = opentelemetry::Context::current();
    let span = context.span();
    let span_context = span.span_context();
    span_context
        .is_valid()
        .then(|| span_context.trace_id().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::{
        InstrumentationScope,
        trace::{Event, SpanContext, SpanId, SpanKind, TraceFlags, TraceId, TraceState},
    };
    use opentelemetry_sdk::trace::SpanLinks;
    use std::{
        borrow::Cow,
        io,
        sync::{Arc, Mutex},
        time::SystemTime,
    };

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for Captured {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self {
            self.clone()
        }
    }

    fn logged_inside_span(provider: Option<&SdkTracerProvider>) -> String {
        let captured = Captured::default();
        let subscriber = subscriber(EnvFilter::new("info"), provider, captured.clone());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(target: "scope_test", "request");
            let _entered = span.enter();
            tracing::info!(target: "scope_test", "handled");
        });
        String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn log_lines_carry_the_trace_id_only_when_spans_are_exported() {
        let exported = logged_inside_span(Some(&SdkTracerProvider::builder().build()));
        let trace_id = exported
            .strip_prefix("trace_id=")
            .and_then(|rest| rest.split_once(' '))
            .map(|(trace_id, _)| trace_id)
            .unwrap_or_default();
        assert_eq!(trace_id.len(), 32, "{exported}");
        assert!(trace_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(exported.ends_with("handled\n"), "{exported}");

        let local = logged_inside_span(None);
        assert!(!local.contains("trace_id="), "{local}");
        assert!(local.ends_with("handled\n"), "{local}");
    }

    #[test]
    fn exported_spans_keep_only_reviewed_attributes() {
        let mut events = SpanEvents::default();
        events.events.push(Event::new(
            "loaded acme/private-app/README.md",
            SystemTime::UNIX_EPOCH,
            vec![KeyValue::new("path", "README.md")],
            0,
        ));
        let mut span = SpanData {
            span_context: SpanContext::new(
                TraceId::from(1),
                SpanId::from(1),
                TraceFlags::SAMPLED,
                false,
                TraceState::default(),
            ),
            parent_span_id: SpanId::INVALID,
            parent_span_is_remote: false,
            span_kind: SpanKind::Server,
            name: Cow::Borrowed("GET /repos/{owner}/{repo}"),
            start_time: SystemTime::UNIX_EPOCH,
            end_time: SystemTime::UNIX_EPOCH,
            attributes: vec![
                KeyValue::new("http.route", "/repos/{owner}/{repo}"),
                KeyValue::new("url.path", "/repos/acme/private-app"),
                KeyValue::new("url.query", "token=secret"),
                KeyValue::new("user_agent.original", "scope/1.0"),
                KeyValue::new("exception.message", "repository acme/private-app failed"),
            ],
            dropped_attributes_count: 0,
            events,
            links: SpanLinks::default(),
            status: Status::error("repository acme/private-app failed"),
            instrumentation_scope: InstrumentationScope::builder("scope").build(),
        };
        keep_allowed_attributes(&mut span);
        assert_eq!(
            span.attributes,
            vec![KeyValue::new("http.route", "/repos/{owner}/{repo}")]
        );
        assert_eq!(span.status, Status::error(""));
        assert!(span.events.is_empty());
    }
}
