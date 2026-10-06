# Tracing

Railway traces every request at its edge. The Rust services continue those
traces with OpenTelemetry so a slow request shows which route, query, call, or
git command took the time. The rollout plan is
[Railway tracing and OpenTelemetry](https://web-production-4f13c9.up.railway.app/plans/scopevcs.com/railway-tracing-otel-plan).

## How services export

Each Rust binary calls `scope_service_runtime::init_telemetry` before starting
its async runtime and `Telemetry::shutdown` after the runtime returns, which
flushes the last batch of spans. Export switches on only when
`OTEL_EXPORTER_OTLP_ENDPOINT` is set. Railway sets it, with the service name,
version, and headers, on the first deploy after tracing is enabled for a service
in an environment. Without it, local runs and tests log exactly as before.

`request_tracing` wraps every router served through `scope_service_runtime::serve`,
and the API router directly. It reads the incoming `traceparent`, names the
server span after the matched route, and skips `/healthz` and `/readyz`.

When spans are exported, every log line written inside a traced request starts
with `trace_id=<id>`. Paste the ID into the Railway Traces page to open the
request.

## What spans may contain

Anyone with access to the Railway project can read span attributes, so the
exporter keeps only the attributes in `EXPORTED_ATTRIBUTES` and drops span
events and error descriptions. Logs keep those details and link back by trace
ID. Spans use internal IDs, never repository owners, names, file paths, query
strings, tokens, SQL parameters, or git arguments. A new attribute is exported
only after it is added to that list.

`EXPORTED_SPANS` selects which `tracing` targets become spans: Scope's crates and
the HTTP server spans. It keeps out the exporter's own HTTP client and query
logging.

## Checking a trace locally

```bash
docker run --rm -p 16686:16686 -p 4318:4318 jaegertracing/jaeger:latest
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
```

Start a service with that variable, send it a request, and open
http://localhost:16686.
