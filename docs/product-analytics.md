# Product analytics

Scope captures product outcomes with PostHog. The browser sends explicit pageviews
and sanitized diagnostics through the web service's `/e` endpoint. API, worker
and the CLI download service use `scope-product-analytics` to send outcomes
directly to PostHog. Analytics delivery is best effort and does not gate product mutations.

## Runtime configuration

Analytics is disabled unless explicitly configured. Web and backend use
`SCOPE_ANALYTICS_ENVIRONMENT=production|test` and `POSTHOG_PROJECT_TOKEN`.
`production` is rejected when Railway's environment name is present and differs
from `production`; `test` is rejected in Railway production. Local development
leaves these variables unset. A test deployment must use its own PostHog project
token. The same token also turns on log and trace export for every service; see
[observability](observability.md).

The web service additionally requires `SCOPE_ANALYTICS_ORIGIN`, an exact HTTP(S)
origin without a path, query, credentials, or fragment. Requests on other origins
receive disabled configuration and cannot forward analytics. Railway terminates
TLS at its edge, so on Railway the request origin is compared as HTTPS, the same
rule the server-function CSRF check uses. The production
origin is `https://scopevcs.com`; configure the actual canonical origin if hosting
changes. GET `/e/config` returns only public SDK configuration or `null`, with
`Cache-Control: no-store`. This policy runs at request time, so the same built
artifact can be promoted across environments.

The browser sends one JSON event per POST `/e/e/`, which forwards to
`https://us.i.posthog.com/e/`. The proxy has a fixed destination and bounded body
size and timeouts. It reapplies the browser event allowlist, pins the project
token and deployment context, removes application cookies, authorization and raw
referrers, and does not cache submissions or log their bodies. Compressed
requests, batches, unknown events and other paths under `/e` are rejected.
Client query parameters cannot override ingestion behavior. Replay, broad
autocapture and remote feature loading remain disabled.

The API, worker and CLI download service accept `POSTHOG_HOST`, defaulting to PostHog US ingestion.
They share the web project's token and region in production. They do not use the
web proxy. The project token is public; a personal API key must never be placed in
application configuration.

`SCOPE_ANALYTICS_RELEASE` is the immutable source SHA embedded in prepared API,
worker, web and CLI download images. Do not override it with a persistent
Railway variable. Explicit local test configurations may omit it. Events
include `environment`, `source`, and the available `release` for diagnosis.

## Collection contract

- Identify people with internal `scope_usr_…` IDs. Reset on sign-out and isolate
  account changes. Backend event capture has no browser preference input.
- The browser client stores nothing on the device: no cookies, `localStorage`
  or other storage. It keeps the anonymous ID and the Scope user ID in memory,
  so every full page load starts with a new random anonymous ID. Anonymous
  unique-visitor counts therefore count page loads rather than returning
  visitors. Signed-in users are still attributed to their `scope_usr_…` ID once
  the viewer identity resolves. `$identify` with `$anon_distinct_id` is sent only
  when events were already captured under the anonymous ID in the same page
  lifetime, for example a visitor who browses and then signs in. Otherwise the
  client switches to the user ID without an `$identify` event.
- Browser analytics is disabled entirely when Global Privacy Control
  (`navigator.globalPrivacyControl === true`) or Do Not Track is set.
- Analytics `repository_id` is the opaque `repoi_…` incarnation ID, **not** the
  domain repository ID, which contains `owner/name`. Deleting and recreating a
  repository produces a new analytics entity.
- Request, discussion, run, and attempt IDs correlate work across actors. An
  author's submit and a maintainer's merge remain separate actor events joined
  by request ID.
- Exclude names, email addresses, repository names, source code, file paths,
  request titles, discussion content, command arguments and raw error messages.
  Browser events pass through an allowlist that rejects unknown event types and
  fields. Browser events leave the client with GeoIP disabled. The web proxy
  enables enrichment only after explicit server activation and a valid Railway
  client address. Backend GeoIP, replay, broad autocapture and person-property
  mutations remain disabled.
- Failures use stable operation and reason classifications. A transport failure
  does not invent a successful domain outcome. Idempotent/no-op transitions do
  not emit a second success event.
- `cli:artifact_download` is recorded by the CLI download service each time it
  serves a CLI archive to a GET request, under the system actor with `target`,
  `os` and `arch`. HEAD probes, checksum files and missing artifacts are not
  counted. The installer downloads
  the same archive for first installs and upgrades, so the count includes both;
  it identifies neither people nor machines.
- `workflow:attempt_start` and `workflow:attempt_complete` describe admitted
  attempts, including provisioning and retries. `duration_ms` measures the
  attempt. `run_result` is present only when the overall run is terminal. Count
  distinct `run_id` values with `run_result` to avoid treating a successful
  intermediate job as a completed run. System activity uses an explicit system
  actor when no initiating user exists.

The event constructors and browser privacy module are the authoritative schema.
The shared analytics crate owns backend context and transport; use cases own
when an outcome has committed. No transactional analytics outbox is introduced.

## Approximate location analytics

Location collection defaults off. Set `SCOPE_ANALYTICS_GEOIP=enabled` on the web
service only after the release prerequisites below are complete. The flag is
server-only and does not change the browser configuration or storage contract.
The proxy trusts only Railway's `X-Real-IP` when `RAILWAY_ENVIRONMENT_ID` is set.
It validates a single IPv4 or IPv6 address and forwards it as `$ip` and a fresh
`X-Forwarded-For`. Incoming forwarding chains, body IPs and location properties
are discarded. Missing or invalid addresses, other hosts, an unset flag, GPC and
DNT leave GeoIP disabled, so PostHog cannot substitute the web server's location.
The proxy uses the canonical public HTTPS origin on Railway.

`dev/analytics/geoip.hog` replaces the project's default GeoIP transformation.
It stores only country name/code, the first region name/code and city name on
events. It does not set person properties, retain raw IPs, coordinates, postal
codes or other geographic fields, or print lookup inputs/results to logs.
Enrichment is limited to explicitly enabled browser events; API, worker and CLI
outcomes remain unenriched. Locations describe the connection and can be wrong
for VPNs, mobile networks and shared gateways. Unknown locations remain visible
in reports. Anonymous counts are pageviews, not distinct people.

Preview the project configuration without making changes:

```sh
node --env-file=/absolute/path/to/private-posthog.env dev/analytics/configure-geoip.mjs --preview
```

Apply using the same private environment-file convention as report syncing:

```sh
node --env-file=/absolute/path/to/private-posthog.env dev/analytics/configure-geoip.mjs --apply
```

The operator key needs project read/write and Hog function read/write scopes.
The command requires exactly one default GeoIP transformation, enables project
IP discard before updating that owner, and verifies the live configuration.
It never enables the application's collection switch. Use a separate test
project and token for ingestion verification. The Scope test project is
`678934` at `https://us.posthog.com/project/678934`.

CI tests the proxy, configuration command and reports without PostHog credentials.
It does not execute the Hog transformation. After editing `geoip.hog`, apply it
to the isolated test project with the command above using a test-project
environment file, then repeat the ingestion verification below.

Before production activation:

1. Verify `anonymize_ips=true`, the exact checked-in transformation is live and
   enabled, and there are no later transformations adding geographic fields.
2. Test a synthetic browser event through the proxy and inspect the ingested
   event in the test project. Check country/region/city, absence of `$ip`,
   coordinates, postal codes and person-property updates, disabled events,
   forged forwarding headers, GPC and DNT.
3. Verify Railway overwrites a forged `X-Real-IP` on the actual public web
   service before trusting that ingress. Do not add a fallback to an arbitrary
   incoming `X-Forwarded-For` chain.
4. Assess and record the lawful basis for the expanded collection, including
   whether applicable analytics rules require consent. Publish the planned
   effective date and notify account holders in advance as promised in the
   privacy policy. Publish the active wording when collection starts.
5. Enable `SCOPE_ANALYTICS_GEOIP=enabled` on the web service through the normal
   release process. Inspect a known journey after release, then sync reports.

Rollback unsets the web switch. Keep IP discard enabled. Merges do not deploy
or activate collection; future release verification must check these conditions.

## Reports

`dev/analytics/reports.mjs` defines seven reports on one **Scope product outcomes**
dashboard. The production dashboard is [Scope product outcomes](https://us.posthog.com/project/570304/dashboard/2096501).

The reports cover:

1. Browser pageviews over 30 days by approximate country, region and city,
   including unknown locations. This counts pageviews rather than unique people.
2. Account creation to CLI authentication to first push, in order within seven
   days. The cohort includes accounts created 7 to 37 days ago so each account
   has a full observation window.
3. Request completion and mean time to discussion/merge, joined by request ID
   across actors. Open requests remain in the completion denominator.
4. Weekly active repository incarnations and the share also active the preceding
   week. Activity means pushes, submitted/revised/merged requests and discussion
   work. The current incomplete week is excluded.
5. Weekly active and returning contributors, using internal actor IDs and earlier
   observed activity.
6. Workflow attempt outcomes and duration, deduplicated by attempt ID.
7. Completed workflow run outcomes and duration, deduplicated by run ID and gated
   on terminal `run_result`.

All reports select `environment='production'`. Internal activity exclusions are
report configuration, not mutable person properties or capture-time guesses.
Set `SCOPE_ANALYTICS_EXCLUDED_USER_IDS` and
`SCOPE_ANALYTICS_EXCLUDED_REPOSITORY_IDS` to comma-separated opaque IDs when
preparing and applying reports. Leave empty only when internal activity should
be included. User exclusions cover actor events; repository exclusions remove
workflow/system activity for those repositories too.

Preview the exact definitions without credentials or network requests:

```sh
node dev/analytics/sync-reports.mjs --preview
```

To apply, provide `POSTHOG_PERSONAL_API_KEY` and `POSTHOG_PROJECT_ID` through your
local environment. The personal key needs query read, dashboard read/write and
insight read/write access to the selected project. `POSTHOG_APP_HOST` defaults to
`https://us.posthog.com`; the EU app host is also supported. This is an operator
command and does not run during application startup or deployment.

```sh
node --env-file=/absolute/path/to/private-posthog.env dev/analytics/sync-reports.mjs --apply
```

The command executes every query before modifying dashboards, then updates
insights by their `scope-product-report:` tags. Running it again updates the same
reports. It preserves unrelated dashboard associations and never deletes reports.
A validation failure stops before writes. Saved definitions and a successful
capture response do not prove live event ingestion; inspect a known journey in
PostHog after the release.

References: [PostHog proxy routing](https://posthog.com/docs/advanced/proxy/proxy-reference),
[insights API](https://posthog.com/docs/api/insights),
[dashboards API](https://posthog.com/docs/api/dashboards), and
[query API](https://posthog.com/docs/api/query).

## Release checks

1. Before releasing this code, configure production API, worker, web and CLI
   downloads with `SCOPE_ANALYTICS_ENVIRONMENT=production` and the same
   `POSTHOG_PROJECT_TOKEN`. Configure the web origin. Keep staging unset or use
   an explicitly separate test project. Confirm the API's host is the US region
   used by the browser proxy.
2. Deploy through the normal release workflow. Prepared images carry their exact
   source revision. Code merges do not deploy changes.
3. Inspect the browser's same-origin capture requests with blockers enabled and
   disabled. Verify actual pageview, identify and backend outcome events in the
   selected PostHog project. Inspect sanitized payloads and correlate a request
   performed by two users.
4. Check proxy latency/errors and the API/worker delivery warnings. Check known
   test counts and exclude internal journeys before relying on the reports.
5. After the new release is verified, delete obsolete GitHub variables
   `VITE_POSTHOG_HOST` and `VITE_POSTHOG_PROJECT_TOKEN`. Their values are no longer
   read by this source. Preserve API `POSTHOG_HOST`.

Rollback uses the existing release workflow or disables analytics. There is no
secondary direct-browser send path. A same-origin proxy reduces domain-based
blocking; it does not guarantee delivery through every blocker.
