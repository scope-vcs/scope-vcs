use opentelemetry::{global, trace::TraceContextExt as _};
use std::collections::HashMap;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

const TRACEPARENT: &str = "traceparent";
const QUEUED_TRACE_IDS: &str = "scope.job.queued_trace_ids";

pub(super) fn install() {
    scope_postgres::job_trace::install(scope_postgres::job_trace::JobTracing {
        current_traceparent,
        link_queued_traces,
    });
}

fn current_traceparent() -> Option<String> {
    let context = tracing::Span::current().context();
    if !context.span().span_context().is_sampled() {
        return None;
    }
    let mut carrier = HashMap::new();
    global::get_text_map_propagator(|propagator| propagator.inject_context(&context, &mut carrier));
    carrier.remove(TRACEPARENT)
}

fn link_queued_traces(span: &tracing::Span, traceparents: &[&str]) {
    let queued = traceparents
        .iter()
        .map(|traceparent| {
            let carrier = HashMap::from([(TRACEPARENT.to_owned(), (*traceparent).to_owned())]);
            global::get_text_map_propagator(|propagator| propagator.extract(&carrier))
                .span()
                .span_context()
                .clone()
        })
        .filter(|context| context.is_valid())
        .collect::<Vec<_>>();
    if queued.is_empty() {
        return;
    }
    let trace_ids = queued
        .iter()
        .map(|context| context.trace_id().to_string())
        .collect::<Vec<_>>()
        .join(",");
    span.set_attribute(QUEUED_TRACE_IDS, trace_ids);
    for context in queued {
        span.add_link(context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::{AllowedAttributes, RequestsAndJobs};
    use opentelemetry::{
        KeyValue,
        trace::{SpanId, TracerProvider as _},
    };
    use opentelemetry_sdk::{
        propagation::TraceContextPropagator,
        trace::{InMemorySpanExporter, Sampler, SdkTracerProvider},
    };
    use tracing_subscriber::layer::SubscriberExt as _;

    #[test]
    fn a_claimed_job_links_every_sampled_trace_that_queued_it() {
        global::set_text_map_propagator(TraceContextPropagator::new());
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::ParentBased(Box::new(RequestsAndJobs)))
            .with_simple_exporter(AllowedAttributes(exporter.clone()))
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        let (queued, unsampled) = tracing::subscriber::with_default(subscriber, || {
            let queued = [(), ()].map(|()| {
                tracing::info_span!("request", otel.kind = "server")
                    .in_scope(current_traceparent)
                    .unwrap()
            });
            let unsampled =
                tracing::info_span!("poll", otel.kind = "client").in_scope(current_traceparent);
            let job = tracing::info_span!(parent: None, "job", otel.kind = "consumer");
            link_queued_traces(&job, &queued.each_ref().map(String::as_str));
            (queued, unsampled)
        });

        let spans = exporter.get_finished_spans().unwrap();
        let requests = spans
            .iter()
            .filter(|span| span.name == "request")
            .map(|span| (span.span_context.trace_id(), span.span_context.span_id()))
            .collect::<Vec<_>>();
        let job = spans.iter().find(|span| span.name == "job").unwrap();
        assert_eq!(
            queued.to_vec(),
            requests
                .iter()
                .map(|(trace_id, span_id)| format!("00-{trace_id}-{span_id}-01"))
                .collect::<Vec<_>>()
        );
        assert_eq!(unsampled, None);
        assert_eq!(job.parent_span_id, SpanId::INVALID);
        assert!(
            requests
                .iter()
                .all(|(trace_id, _)| *trace_id != job.span_context.trace_id())
        );
        assert_eq!(
            job.links
                .iter()
                .map(|link| (link.span_context.trace_id(), link.span_context.span_id()))
                .collect::<Vec<_>>(),
            requests
        );
        assert_eq!(
            job.attributes,
            [KeyValue::new(
                QUEUED_TRACE_IDS,
                format!("{},{}", requests[0].0, requests[1].0)
            )]
        );
    }
}
