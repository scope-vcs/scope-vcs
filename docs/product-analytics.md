# Product analytics

Scope captures product outcomes with PostHog. The browser sends explicit pageviews
and sanitized diagnostics through the web service's `/e` endpoint. API and worker
use `scope-product-analytics` to send committed domain outcomes directly to
PostHog. Analytics delivery is best effort and does not gate product mutations.

## Runtime configuration

Analytics is disabled unless explicitly configured. Web and backend use
`SCOPE_ANALYTICS_ENVIRONMENT=production|test` and `POSTHOG_PROJECT_TOKEN`.
`production` is rejected when Railway's environment name is present and differs
from `production`; `test` is rejected in Railway production. Local development
leaves these variables unset. A test deployment must use its own PostHog project
token.

The web service additionally requires `SCOPE_ANALYTICS_ORIGIN`, an exact HTTP(S)
origin without a path, query, credentials, or fragment. Requests on other origins
receive disabled configuration and cannot forward analytics. The production
origin is `https://scopevcs.com`; configure the actual canonical origin if hosting
changes. GET `/e/config` returns only public SDK configuration or `null`, with
`Cache-Control: no-store`. This policy runs at request time, so the same built
artifact can be promoted across environments.

Browser initialization uses `api_host: '/e'` and
`ui_host: 'https://us.posthog.com'`. The pinned SDK submits to POST `/e/e/`, which
forwards to `https://us.i.posthog.com/e/`. The proxy has a fixed destination and
bounded body size and timeouts. It removes application cookies, authorization,
raw referrer and client-IP forwarding headers, preserves the capture encoding
and SDK query, and does not cache submissions or log their bodies. Other paths
under `/e` are rejected. New SDK endpoints require an explicit routing change and
a browser test; replay and remote feature loading remain disabled.

The API and worker accept `POSTHOG_HOST`, defaulting to PostHog US ingestion.
They share the web project's token and region in production. They do not use the
web proxy. The project token is public; a personal API key must never be placed in
application configuration.

`SCOPE_ANALYTICS_RELEASE` is the immutable source SHA embedded in prepared API,
worker, and web images. Do not override it with a persistent Railway variable.
Explicit local test configurations may omit it. Events include `environment`,
`source`, and the available `release` for diagnosis.

## Collection contract

- Identify people with internal `scope_usr_…` IDs. Reset on sign-out and isolate
  account changes. Browser Do Not Track remains respected; backend event capture
  has no browser preference input.
- Analytics `repository_id` is the opaque `repoi_…` incarnation ID, **not** the
  domain repository ID, which contains `owner/name`. Deleting and recreating a
  repository produces a new analytics entity.
- Request, discussion, run, and attempt IDs correlate work across actors. An
  author's submit and a maintainer's merge remain separate actor events joined
  by request ID.
- Exclude names, email addresses, repository names, source code, file paths,
  request titles, discussion content, command arguments and raw error messages.
  Browser events pass through an allowlist that rejects unknown event types and
  fields. GeoIP enrichment, replay, broad autocapture and person-property
  mutations remain disabled.
- Failures use stable operation and reason classifications. A transport failure
  does not invent a successful domain outcome. Idempotent/no-op transitions do
  not emit a second success event.
- `workflow:attempt_start` and `workflow:attempt_complete` describe admitted
  attempts, including provisioning and retries. `duration_ms` measures the
  attempt. `run_result` is present only when the overall run is terminal. Count
  distinct `run_id` values with `run_result` to avoid treating a successful
  intermediate job as a completed run. System activity uses an explicit system
  actor when no initiating user exists.

The event constructors and browser privacy module are the authoritative schema.
The shared analytics crate owns backend context and transport; use cases own
when an outcome has committed. No transactional analytics outbox is introduced.

## Reports

`dev/analytics/reports.mjs` defines six reports on one **Scope product outcomes**
dashboard. The production dashboard is [Scope product outcomes](https://us.posthog.com/project/570304/dashboard/2096501).

The reports cover:

1. Account creation to CLI authentication to first push, in order within seven
   days. The cohort includes accounts created 7 to 37 days ago so each account
   has a full observation window.
2. Request completion and mean time to discussion/merge, joined by request ID
   across actors. Open requests remain in the completion denominator.
3. Weekly active repository incarnations and the share also active the preceding
   week. Activity means pushes, submitted/revised/merged requests and discussion
   work. The current incomplete week is excluded.
4. Weekly active and returning contributors, using internal actor IDs and earlier
   observed activity.
5. Workflow attempt outcomes and duration, deduplicated by attempt ID.
6. Completed workflow run outcomes and duration, deduplicated by run ID and gated
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

1. Before releasing this code, configure production API, worker and web with
   `SCOPE_ANALYTICS_ENVIRONMENT=production` and the same
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
