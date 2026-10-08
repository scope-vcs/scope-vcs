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

## Which traces are kept

A span with a parent follows its parent's sampling decision, so every span
inside an edge request is kept. A span without a parent starts a trace only
when it is a server span or a claimed job's consumer span. Store queries and
outbound calls made by idle worker polls have no parent and are dropped, with
everything nested under them, so polling cannot exhaust Railway's span quota.

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

## Controlled production activation

`.github/scripts/railway-tracing.mjs` owns the service tracing settings for
previews and production. It enables tracing for every application service and
the maintenance runtime, enables web auto-instrumentation, and excludes
PostgreSQL. Preview provisioning calls the same owner before deployment.
Production's monitored backend and web deployment wrapper calls it before
starting the release transition, under the existing Release concurrency lock.
An unconfirmed Railway mutation blocks activation. Mutations only change
tracing settings; this owner never starts deployments or supplies exporter
credentials. Railway injects those credentials on each subsequent deployment.

Use a controlled Release with scope `all` for the initial rollout. A selective
release only activates export on services it actually deploys; changing a
setting alone does not prove runtime coverage. Recheck carried services after
a selective release. Pushes and merges do not activate tracing. Preserve the
existing release recovery procedure if activation fails; do not redeploy
services individually to finish this rollout.

## Deployed coverage verification

Run these read-only commands after the controlled release, using the pinned
Railway CLI and the manifest in the released checkout. Disable shell tracing.
The runtime probe reports presence only, never endpoint or header values.
It checks a running instance; inspect every active instance when replicas
were replaced unevenly.

```bash
set +x
manifest=.github/deployment-services.json
project_id="$(jq -er '.railway.projectId' "$manifest")"
environment_id="$(jq -er '.environments.production.environmentId' "$manifest")"
for component in api run-worker cache git-router media-api media-worker web; do
  service_id="$(jq -er --arg component "$component" '.services[$component].id' "$manifest")"
  printf '%s: ' "$component"
  railway ssh --project "$project_id" --environment "$environment_id" \
    --service "$service_id" -- sh -ceu \
    'if [ -n "${OTEL_EXPORTER_OTLP_ENDPOINT:-}" ]; then printf "exporter present\n"; else printf "exporter absent\n"; exit 1; fi'
done
```

Open the production project in
[Railway](https://railway.com/project/45dd67fa-6d69-48ad-9680-1313d41b4490?environmentId=2f21f6b6-6817-4338-9e08-cf18e78b8f46).
Set the trace window to the release completion time onward. Use staging as
a reference, then exercise a repository code read and request/check refresh
through the deployed app. Record the release SHA, deployment IDs, timestamp,
trace ID, matched route, API span, and its database child span. A health probe
cannot prove coverage because health routes are intentionally excluded.

Extract only trace IDs and timestamps from deployment logs for correlation:

```bash
service_id="$(jq -er '.services.api.id' "$manifest")"
railway logs --project "$project_id" --environment "$environment_id" \
  --service "$service_id" --since 15m --lines 500 --json |
  jq -r 'select(.message | test("trace_id=[0-9a-f]{32}")) |
    [.timestamp, (.message | capture("trace_id=(?<id>[0-9a-f]{32})").id)] | @tsv'
```

Find one of those IDs in Railway Traces and confirm it belongs to the exercised
request and the expected deployed service. Inspect cache, Git router and media
spans when the exercised flow reaches them. For workers, inspect a real claimed
job and its consumer span; idle polls intentionally produce no root traces.
For web, confirm a real runtime request emits a span from `scope-web` rather
than counting Railway edge spans as proof of web auto-instrumentation. If the
web runtime exports no useful spans despite configured instrumentation, record
that gap explicitly. Settings, injected variables, and a successful deployment
are prerequisites; only received spans establish coverage.

Retain an allowlisted evidence table with service, deployment ID, exercised
operation, time window, trace ID, observed span kinds, and coverage gaps. Do not
copy raw logs, exporter headers, tokens, repository paths, or request payloads
into release evidence. Record request/job exposure counts alongside errors.

## Follow-up after release

When the release actually completes, create T3 checks for one hour and 24 hours
after its completion time. Anchor both to that release SHA and deployment IDs;
preparation and merge time are not the release time. At each check, review the
transaction-isolation error and GitHub-check constraint signatures, browser
errors grouped by release and route, and request/job counts. Separate
maintenance 503s and handled cancellations from application failures. Repeat
the deployed span/log correlation above for representative traffic and report
coverage gaps. A quiet period without exercised flows is not proof of a fix.
A recurring error needs captured context and reproduction before closure.

## Checking a trace on a development host

Bind Jaeger's UI to the host's Tailscale address so another device can reach it.
The exporter can use loopback because it runs on the same host.

```bash
tailscale_ip="$(tailscale ip -4)"
docker run --rm -p "$tailscale_ip:16686:16686" -p 127.0.0.1:4318:4318 jaegertracing/jaeger:latest
export OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318
```

Start a service with that variable, send it a request, and open
`http://<tailscale-ip>:16686` from the other device. Verify the actual address
with `tailscale ip -4` before sharing the link.
