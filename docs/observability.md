# Observability

Scope's services send their logs and traces to PostHog over OpenTelemetry, so a
slow or failed request opens with its route, queries, outbound calls, git
commands, and log lines in one place. Railway keeps build and deploy logs,
resource metrics, and each service's stdout, which still works when export does
not.

## How services export

Each Rust binary calls `scope_service_runtime::init_telemetry` with its
deployment component name before starting its async runtime, and
`Telemetry::shutdown` after the runtime returns, which flushes the last batch of
spans and logs. The web service preloads `web/src/server/instrumentation.ts`,
built into its own bundle, with `node --import` before the application starts;
both the image command and the Railway artifact start command include it, and
release extraction rejects a web build without it.
It traces incoming requests, named by their analytics route alias rather than
the raw path, and outgoing `fetch` calls, and applies the same
sampling and attribute rules as the Rust services, and exports server logs
written through `serverLog`. Nitro's close hook flushes the last batch.

Export switches on only when `POSTHOG_PROJECT_TOKEN` is set. Spans go to
`$POSTHOG_HOST/i/v1/traces` and logs to `$POSTHOG_HOST/i/v1/logs`, with the
project token as a bearer token. `POSTHOG_HOST` defaults to
`https://us.i.posthog.com`. Every record carries `service.name`, the release SHA
from `SCOPE_ANALYTICS_RELEASE` as `service.version`, and
`RAILWAY_ENVIRONMENT_NAME` as `deployment.environment.name`. Without the token,
local runs and tests log to stdout only.

`request_tracing` wraps every router served through `scope_service_runtime::serve`,
and the API router directly. It reads the incoming `traceparent`, names the
server span after the matched route, and skips `/healthz` and `/readyz`. The web
server propagates `traceparent` on its calls to the API, so a page request and
the API work it causes share one trace.

When spans are exported, every stdout line written inside a traced request
starts with `trace_id=<id>`, which finds the same request in PostHog.

## Railway tracing stays off

Railway injects `OTEL_EXPORTER_OTLP_*` variables, including exporter headers,
into services that have its tracing enabled. OTLP exporters merge those headers
into every request, so they would send Railway's credentials to PostHog. A
service that sees them logs an error and exports nothing.
`.github/scripts/railway-tracing.mjs` owns the Railway settings: it disables
tracing on every application service and the maintenance runtime, and web
auto-instrumentation, before each production release and during preview
provisioning. An unconfirmed Railway mutation blocks the release. Railway applies
the change on each service's next deployment.

## Which traces are kept

A span with a parent follows its parent's sampling decision, so every span
inside a request is kept. A span without a parent starts a trace only when it is
a server span or a claimed job's consumer span. Store queries and outbound calls
made by idle worker polls have no parent and are dropped, with everything nested
under them, so polling does not consume PostHog's ingestion quota.

## What leaves a service

Anyone with access to the PostHog project can read what is exported.

Spans keep only the attributes in `EXPORTED_ATTRIBUTES`, and the exporter drops
span events and error descriptions. Spans use internal IDs, never repository
owners, names, file paths, query strings, tokens, SQL parameters, or git
arguments. A new attribute is exported only after it is added to that list.
`EXPORTED_SPANS` selects which `tracing` targets become spans.

Logs are exported as written: every event that passes the service's log filter,
`info` and above from Scope's own crates by default, with its fields as
attributes. Log lines can name repositories, paths, and error details, which is
why they are not span attributes. Errors from `outbound_http::send_traced` drop
their URL, so presigned object links and query credentials never reach a log
line. The OpenTelemetry SDK and the HTTP client
crates are never exported, so the exporter cannot log its own requests. PostHog
keeps logs and traces for 14 days by default. Restrict project access
accordingly and record any retention change in the
[data inventory](../legal/data-inventory.md).

## Configuration

Production sets `POSTHOG_PROJECT_TOKEN` on api, run-worker, git-router,
media-api, media-worker, cli-downloads, and web, using the same project as
product analytics.
Product analytics additionally requires `SCOPE_ANALYTICS_ENVIRONMENT`, so
setting the token alone on a service exports telemetry without capturing
product events. Staging and previews leave the token unset unless they are given
a separate PostHog project.

## Deployed coverage verification

Run these read-only checks after the release that carries this export, using
the pinned Railway CLI and the manifest in the released checkout. Disable shell
tracing. The probe reports presence only, never values. It checks one running
instance; inspect every active instance when replicas were replaced unevenly.

```bash
set +x
manifest=.github/deployment-services.json
project_id="$(jq -er '.railway.projectId' "$manifest")"
environment_id="$(jq -er '.environments.production.environmentId' "$manifest")"
for component in api run-worker git-router media-api media-worker cli-downloads web; do
  service_id="$(jq -er --arg component "$component" '.services[$component].id' "$manifest")"
  printf '%s: ' "$component"
  railway ssh --project "$project_id" --environment "$environment_id" \
    --service "$service_id" -- sh -ceu \
    'if [ -n "${OTEL_EXPORTER_OTLP_HEADERS:-}" ]; then echo "railway exporter injected"; exit 1; fi
     if [ -z "${POSTHOG_PROJECT_TOKEN:-}" ]; then echo "posthog token absent"; exit 1; fi
     echo "posthog export configured"'
done
```

Then exercise a repository code read and a request/check refresh through the
deployed app. In PostHog, filter logs and traces by `service.name`,
`deployment.environment.name = production`, and the release SHA. Confirm a web
request span with an API child span and its database span, a claimed worker
job's consumer span, and log lines from each service, including at least one
that carries the trace ID of an exercised request. Health probes cannot prove
coverage because health routes are excluded, and idle workers intentionally
produce no root traces. Settings, variables, and a successful deployment are
prerequisites; only received records establish coverage.

Retain an allowlisted evidence table with service, deployment ID, exercised
operation, time window, trace ID, observed span kinds, and coverage gaps. Do not
copy raw logs, tokens, repository paths, or request payloads into release
evidence. Record request and job counts alongside errors.

## Follow-up after release

When the release actually completes, create T3 checks for one hour and 24 hours
after its completion time. Anchor both to that release SHA and deployment IDs;
preparation and merge time are not the release time. At each check, review
error-level logs by service, browser errors grouped by release and route, and
request/job counts. Separate maintenance 503s and handled cancellations from
application failures. Repeat the span and log correlation above for
representative traffic and report coverage gaps. A quiet period without
exercised flows is not proof of a fix.
