use axum_tracing_opentelemetry::middleware::OtelAxumLayer;
use opentelemetry::{
    Context,
    trace::{Link, SpanKind, Status, TraceId, TraceState},
};
use opentelemetry::{
    KeyValue, global,
    trace::{TraceContextExt as _, TracerProvider as _},
};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{WithExportConfig as _, WithHttpConfig as _};
use opentelemetry_sdk::{
    Resource,
    error::OTelSdkResult,
    logs::SdkLoggerProvider,
    propagation::TraceContextPropagator,
    trace::{
        Sampler, SamplingDecision, SamplingResult, SdkTracerProvider, ShouldSample, SpanData,
        SpanEvents, SpanExporter,
    },
};
use std::{collections::HashMap, fmt, time::Duration};
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    EnvFilter, Layer as _,
    filter::{FilterExt as _, filter_fn},
    fmt::{FmtContext, FormatEvent, FormatFields, MakeWriter, format::Writer},
    layer::SubscriberExt as _,
    registry::LookupSpan,
};

#[cfg(feature = "postgres")]
mod job_trace;

const EXPORTED_SPANS: &str =
    "off,otel::tracing=trace,api=info,worker=info,scope_=info,aws_sdk_s3::operation=debug";
const UNEXPORTED_LOG_TARGETS: [&str; 5] = ["opentelemetry", "reqwest", "hyper", "h2", "tower"];
const DEFAULT_POSTHOG_HOST: &str = "https://us.i.posthog.com";
const INJECTED_EXPORTER_HEADERS: [&str; 3] = [
    "OTEL_EXPORTER_OTLP_HEADERS",
    "OTEL_EXPORTER_OTLP_TRACES_HEADERS",
    "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
];
const UNTRACED_PATHS: [&str; 2] = ["/healthz", "/readyz"];
const EXPORTED_ATTRIBUTES: [&str; 21] = [
    "db.operation.name",
    "db.system.name",
    "git.subcommand",
    "http.request.method",
    "http.response.status_code",
    "http.route",
    "network.protocol.version",
    "process.exit.code",
    "rpc.method",
    "rpc.service",
    "rpc.system",
    "scope.blob.count",
    "scope.change.count",
    "scope.job.attempt",
    "scope.job.id",
    "scope.job.kind",
    "scope.job.queued_trace_ids",
    "scope.landed_request.count",
    "server.address",
    "server.port",
    "url.scheme",
];

#[must_use = "dropping telemetry without shutdown loses the final batch of spans and logs"]
pub struct Telemetry {
    exporters: Option<Exporters>,
}

impl Telemetry {
    pub fn shutdown(self) {
        let Some(exporters) = self.exporters else {
            return;
        };
        if let Err(error) = exporters.tracer.shutdown() {
            tracing::warn!(%error, "flushing exported spans failed");
        }
        if let Err(error) = exporters.logger.shutdown() {
            tracing::warn!(%error, "flushing exported logs failed");
        }
    }
}

struct Exporters {
    tracer: SdkTracerProvider,
    logger: SdkLoggerProvider,
}

pub fn init_telemetry(service: &'static str, log_filter: &str) -> anyhow::Result<Telemetry> {
    let log_filter = service_log_filter(std::env::var("RUST_LOG").ok(), log_filter);
    let destination = PostHogDestination::from_env(|name| std::env::var(name).ok());
    let exporters = match &destination {
        Ok(Some(destination)) => Some(destination.exporters(service)?),
        _ => None,
    };
    install(subscriber(log_filter, exporters.as_ref(), std::io::stdout))?;
    if let Err(InjectedExporterHeaders(variable)) = destination {
        eprintln!(
            "PostHog telemetry export disabled because Railway still injects {variable}; disable Railway tracing for this service"
        );
    }
    Ok(Telemetry { exporters })
}

fn service_log_filter(rust_log: Option<String>, service_default: &str) -> EnvFilter {
    rust_log
        .and_then(|directives| EnvFilter::try_new(directives).ok())
        .unwrap_or_else(|| service_default.into())
}

fn subscriber<W>(
    log_filter: EnvFilter,
    exporters: Option<&Exporters>,
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
                .with_filter(log_filter.clone()),
        )
        .with(exporters.map(|exporters| {
            tracing_opentelemetry::layer()
                .with_tracer(exporters.tracer.tracer("scope"))
                .with_filter(EnvFilter::new(EXPORTED_SPANS))
        }))
        .with(exporters.map(|exporters| {
            OpenTelemetryTracingBridge::new(&exporters.logger).with_filter(log_filter.and(
                filter_fn(|metadata| {
                    !UNEXPORTED_LOG_TARGETS
                        .iter()
                        .any(|target| metadata.target().starts_with(target))
                }),
            ))
        }))
}

fn install(subscriber: impl Subscriber + Send + Sync) -> anyhow::Result<()> {
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}

pub fn request_tracing() -> OtelAxumLayer {
    OtelAxumLayer::default().filter(|path| !UNTRACED_PATHS.contains(&path))
}

#[derive(Debug, PartialEq)]
struct PostHogDestination {
    host: String,
    project_token: String,
    release: Option<String>,
    environment: Option<String>,
}

#[derive(Debug, PartialEq)]
struct InjectedExporterHeaders(&'static str);

impl PostHogDestination {
    fn from_env(
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, InjectedExporterHeaders> {
        let set = |name: &str| {
            env(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        let Some(project_token) = set("POSTHOG_PROJECT_TOKEN") else {
            return Ok(None);
        };
        if let Some(variable) = INJECTED_EXPORTER_HEADERS
            .into_iter()
            .find(|variable| env(variable).is_some())
        {
            return Err(InjectedExporterHeaders(variable));
        }
        Ok(Some(Self {
            host: set("POSTHOG_HOST")
                .unwrap_or_else(|| DEFAULT_POSTHOG_HOST.to_owned())
                .trim_end_matches('/')
                .to_owned(),
            project_token,
            release: set("SCOPE_ANALYTICS_RELEASE"),
            environment: set("RAILWAY_ENVIRONMENT_NAME"),
        }))
    }

    fn endpoint(&self, signal: &str) -> String {
        format!("{}/i/v1/{signal}", self.host)
    }

    fn headers(&self) -> HashMap<String, String> {
        HashMap::from([(
            "Authorization".to_owned(),
            format!("Bearer {}", self.project_token),
        )])
    }

    fn resource(&self, service: &'static str) -> Resource {
        let attributes = [
            ("service.version", &self.release),
            ("deployment.environment.name", &self.environment),
        ];
        Resource::builder()
            .with_service_name(service)
            .with_attributes(
                attributes.into_iter().filter_map(|(key, value)| {
                    value.clone().map(|value| KeyValue::new(key, value))
                }),
            )
            .build()
    }

    fn exporters(&self, service: &'static str) -> anyhow::Result<Exporters> {
        let resource = self.resource(service);
        let spans = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(self.endpoint("traces"))
            .with_headers(self.headers())
            .build()?;
        let logs = opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_endpoint(self.endpoint("logs"))
            .with_headers(self.headers())
            .build()?;
        let tracer = SdkTracerProvider::builder()
            .with_sampler(Sampler::ParentBased(Box::new(RequestsAndJobs)))
            .with_batch_exporter(AllowedAttributes(spans))
            .with_resource(resource.clone())
            .build();
        let logger = SdkLoggerProvider::builder()
            .with_batch_exporter(logs)
            .with_resource(resource)
            .build();
        global::set_text_map_propagator(TraceContextPropagator::new());
        global::set_tracer_provider(tracer.clone());
        #[cfg(feature = "postgres")]
        job_trace::install();
        Ok(Exporters { tracer, logger })
    }
}

#[derive(Clone, Debug)]
struct RequestsAndJobs;

impl ShouldSample for RequestsAndJobs {
    fn should_sample(
        &self,
        _parent_context: Option<&Context>,
        _trace_id: TraceId,
        _name: &str,
        span_kind: &SpanKind,
        _attributes: &[KeyValue],
        _links: &[Link],
    ) -> SamplingResult {
        SamplingResult {
            decision: if matches!(span_kind, SpanKind::Server | SpanKind::Consumer) {
                SamplingDecision::RecordAndSample
            } else {
                SamplingDecision::Drop
            },
            attributes: Vec::new(),
            trace_state: TraceState::default(),
        }
    }
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
    use opentelemetry_sdk::{logs::InMemoryLogExporter, trace::SpanLinks};
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

    fn exporters(logs: &InMemoryLogExporter) -> Exporters {
        Exporters {
            tracer: SdkTracerProvider::builder().build(),
            logger: SdkLoggerProvider::builder()
                .with_simple_exporter(logs.clone())
                .build(),
        }
    }

    fn logged_inside_span(exporters: Option<&Exporters>) -> String {
        let captured = Captured::default();
        let subscriber = subscriber(EnvFilter::new("info"), exporters, captured.clone());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(target: "scope_test", "request");
            let _entered = span.enter();
            tracing::info!(target: "scope_test", "handled");
        });
        String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn an_invalid_log_filter_keeps_the_service_default() {
        let captured = Captured::default();
        let filter = service_log_filter(Some("scope_test=invalid".to_owned()), "scope_test=info");
        tracing::subscriber::with_default(subscriber(filter, None, captured.clone()), || {
            tracing::info!(target: "scope_test", "handled");
        });
        let logged = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(logged.ends_with("handled\n"), "{logged}");
    }

    #[test]
    fn log_lines_carry_the_trace_id_only_when_spans_are_exported() {
        let exported = logged_inside_span(Some(&exporters(&InMemoryLogExporter::default())));
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
    fn logs_after_a_log_crate_level_check_still_reach_every_output() {
        let captured = Captured::default();
        let logs = InMemoryLogExporter::default();
        let exporters = exporters(&logs);
        install(subscriber(
            EnvFilter::new("scope_test=info"),
            Some(&exporters),
            captured.clone(),
        ))
        .unwrap();
        for pass in 0..3 {
            let _ = log::log_enabled!(target: "sqlx::query", log::Level::Debug);
            tracing::info!(target: "scope_test", pass, "pass finished");
        }
        let logged = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert_eq!(logged.matches("pass finished").count(), 3, "{logged}");
        assert_eq!(logs.get_emitted_logs().unwrap().len(), 3);
    }

    #[test]
    fn exported_logs_join_their_request_trace_without_exporter_internals() {
        let logs = InMemoryLogExporter::default();
        let exporters = exporters(&logs);
        let subscriber = subscriber(
            EnvFilter::new("info,reqwest::connect=debug,opentelemetry_sdk=debug"),
            Some(&exporters),
            Captured::default(),
        );
        let trace_id = tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(target: "scope_test", "request", otel.kind = "server");
            let _entered = span.enter();
            tracing::info!(target: "scope_test", "handled");
            tracing::debug!(target: "reqwest::connect", "connecting");
            tracing::debug!(target: "opentelemetry_sdk", "exporting");
            current_trace_id().unwrap()
        });
        let emitted = logs.get_emitted_logs().unwrap();
        let exported: Vec<_> = emitted
            .iter()
            .map(|log| {
                (
                    log.record.target().map(|target| target.to_string()),
                    log.record
                        .trace_context()
                        .map(|context| context.trace_id.to_string()),
                )
            })
            .collect();
        assert_eq!(exported, [(Some("scope_test".to_owned()), Some(trace_id))]);
    }

    #[test]
    fn posthog_export_requires_a_token_and_no_injected_exporter_headers() {
        let env = |variables: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                variables
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };
        assert_eq!(
            PostHogDestination::from_env(env(&[("POSTHOG_HOST", "https://eu.i.posthog.com")])),
            Ok(None)
        );
        let default_host =
            PostHogDestination::from_env(env(&[("POSTHOG_PROJECT_TOKEN", " phc_public ")]))
                .unwrap()
                .unwrap();
        assert_eq!(
            default_host.endpoint("traces"),
            "https://us.i.posthog.com/i/v1/traces"
        );
        assert_eq!(default_host.headers()["Authorization"], "Bearer phc_public");
        let configured_host = PostHogDestination::from_env(env(&[
            ("POSTHOG_PROJECT_TOKEN", "phc_public"),
            ("POSTHOG_HOST", "https://eu.i.posthog.com/"),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            configured_host.endpoint("logs"),
            "https://eu.i.posthog.com/i/v1/logs"
        );
        assert_eq!(
            PostHogDestination::from_env(env(&[
                ("POSTHOG_PROJECT_TOKEN", "phc_public"),
                ("OTEL_EXPORTER_OTLP_HEADERS", "authorization=Bearer railway"),
            ])),
            Err(InjectedExporterHeaders("OTEL_EXPORTER_OTLP_HEADERS"))
        );
    }

    #[derive(Clone, Debug, Default)]
    struct Collected(Arc<Mutex<Vec<String>>>);

    impl SpanExporter for Collected {
        async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
            let mut names = self.0.lock().unwrap();
            names.extend(batch.into_iter().map(|span| span.name.into_owned()));
            Ok(())
        }
    }

    #[test]
    fn traces_start_only_at_requests_and_claimed_jobs() {
        let collected = Collected::default();
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::ParentBased(Box::new(RequestsAndJobs)))
            .with_simple_exporter(collected.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info_span!("claim", otel.kind = "client").in_scope(|| {
                tracing::info_span!("lease", otel.kind = "client").in_scope(|| {});
            });
            tracing::info_span!("git", otel.kind = "internal").in_scope(|| {});
            tracing::info_span!("request", otel.kind = "server").in_scope(|| {
                tracing::info_span!("query", otel.kind = "client").in_scope(|| {});
            });
            tracing::info_span!("job", otel.kind = "consumer").in_scope(|| {
                tracing::info_span!("lease", otel.kind = "client").in_scope(|| {});
            });
        });
        let mut names = collected.0.lock().unwrap().clone();
        names.sort();
        assert_eq!(names, ["job", "lease", "query", "request"]);
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
                KeyValue::new("db.operation.name", "repo_live_file_with_landing_content"),
                KeyValue::new("db.system.name", "postgresql"),
                KeyValue::new("git.subcommand", "show"),
                KeyValue::new("http.route", "/repos/{owner}/{repo}"),
                KeyValue::new("process.exit.code", 0),
                KeyValue::new("rpc.method", "GetObject"),
                KeyValue::new("rpc.service", "S3"),
                KeyValue::new("rpc.system", "aws-api"),
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
            vec![
                KeyValue::new("db.operation.name", "repo_live_file_with_landing_content"),
                KeyValue::new("db.system.name", "postgresql"),
                KeyValue::new("git.subcommand", "show"),
                KeyValue::new("http.route", "/repos/{owner}/{repo}"),
                KeyValue::new("process.exit.code", 0),
                KeyValue::new("rpc.method", "GetObject"),
                KeyValue::new("rpc.service", "S3"),
                KeyValue::new("rpc.system", "aws-api"),
            ]
        );
        assert_eq!(span.status, Status::error(""));
        assert!(span.events.is_empty());
    }
}
