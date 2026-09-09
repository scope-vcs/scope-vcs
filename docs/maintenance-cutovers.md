# Maintenance migration recovery

Migration impact is declared once in `crates/scope-postgres/src/migrations/mod.rs`.
Use `Online` only when old and new runtime contracts can overlap safely. Renames,
removals, rewrites, protocol resets, and changed invariants require
`MaintenanceRequired`. Do not add dual readers or writers to avoid a cutover.

Production builds and publishes release images before closing metadata writers.
The prepared manifest pins the source revision and each Railway service's image
digest. Maintenance requires prepared API, worker, cache, and router artifacts.
Activation consumes those images without a build. A production maintenance cutover
requires an explicit positive outage budget agreed from staging measurements.
Recovery proceeds even when that budget has expired so it can restore service;
availability reporting still records the full outage and budget failure.
A fresh migration plan must match the baseline immediately before closure.

The deployment workflow persists closure intent as a GitHub deployment in
`production/cutover`. Its payload contains the prepared manifest, baseline plan,
and previous deployment identities. Status records mark each phase before its
mutations. GitHub timestamps retain the phase history if the runner is cancelled
or killed; the workflow summary also reports phase durations. An unresolved
record blocks ordinary deployments, including a deployment of a newer revision.

The cutover closes API, worker, and cache, acquires the database writer fence,
applies migrations, verifies the exact ledger, and runs the current idempotent
backfills before activating the prepared services. Keep command ordering and the
active backfill list in `.github/scripts/deploy-backend-railway.sh` and
`.github/scripts/release-cutover.sh`, where the deployment tests exercise them.

## Recovery rule

Dispatch the production workflow on `main` with the cutover deployment ID and
source SHA printed by the failed run. The workflow uses the current main orchestration and extracts the maintenance binary from the recorded API image digest without
starting its container. Its contents must match the manifest's recorded SHA-256:

```sh
gh workflow run scope-production-deploy.yml --ref main \
  -f recover_cutover_id=DEPLOYMENT_ID \
  -f recover_source_sha=FULL_SHA \
  -f maintenance_budget_seconds=APPROVED_SECONDS
```

Recovery restores
the recorded manifest and rejects a different source or image set. It closes
all current metadata writers again, covering a runner killed during shutdown or
a partial activation, then reads the ledger.

An exact ledger proceeds through verification and backfills to forward
activation. An unchanged baseline resumes the pinned migration. A different or
unreadable ledger leaves writers closed. Recovery never builds replacement
images and never chooses artifacts from a newer revision.

The successful migration transaction is the point of no return. Ordinary failure
handling restarts previous deployments only after a fresh ledger read proves
the baseline is unchanged. Before `apply` starts, the workflow records the
`applying` phase durably. A lost response or lost runner cannot erase the fact
that the transaction might have committed. The record remains unresolved until
all forward services are healthy or verified restoration succeeds.

The database fence is the final concurrency boundary. Every API, worker, and
cache database connection holds its shared side for the session. A writer racing
shutdown either makes maintenance refuse or drains before the exclusive fence
is acquired. Runtime startup verifies the exact schema before opening its writer
pool, so an old binary cannot write after a committed migration.

## Read-only inspection

```text
scope-maintenance plan
scope-maintenance verify
node .github/scripts/production-deployment-progress.mjs cutover-read --id DEPLOYMENT_ID --source-sha FULL_SHA
```

`plan` and `verify` emit JSON. `verify` succeeds only when the database ledger
exactly matches the binary. Run `scope-maintenance --help` for maintenance and
backfill commands. Production cutovers should use the deployment workflow.

## Readiness and availability rollout

Enable the readiness rollout before enabling production transition monitoring.
The current web release must already answer `/readyz`; the monitor deliberately
fails its baseline when that endpoint is missing. After reviewing the staging
proof and approving production rollout, activate the prepared web image using
`deploy-railway.sh` with its source SHA and prepared manifest. Confirm `/readyz`
and the effective deployment manifest before enabling the production workflow.
This is the one-time rollout order; there is no fallback readiness route.

Ordinary production activation requires a successful staging rehearsal of the
same immutable image digests. The rehearsal makes three transitions, checks
finite requests once per second, and retains an open repository tab across each
transition. Its 60-second baseline must pass before activation; monitoring lasts
120 seconds after exact predecessor teardown. The full staging rehearsal also
pushes a fixture update and waits for that update to appear without a refresh.
Production uses the public `adamblumoff/pagent` README fixture; staging uses the
seeded `dev/update-demo` repository.

A complete application manifest fences writers, migrates, and resets release-proof
fixtures before rehearsal. A partial manifest requires healthy, seeded release-proof
with the candidate schema already applied. For a migration candidate with a partial
manifest, run the full staging rehearsal first, then import the production artifacts. Agree the production
outage budget from the maintenance exercise and supply
`maintenance_budget_seconds`; its default of zero prevents a new maintenance
cutover while ordinary releases continue to work.

Dispatch staging on `main` to test an arbitrary candidate SHA. A manual branch
dispatch is restricted to that branch's exact SHA and explicitly selects that
branch's orchestration. Candidate compilation receives read-only repository
permissions. Image preparation uses the selected trusted orchestration and
Dockerfiles, treating candidate archives as data; unsafe archive links and paths
are rejected before extraction. Trusted staging setup configures registry access
before candidate code is introduced, without passing registry secrets into that
code.

## Release image storage

Release images use private GHCR packages, addressed by digest. The package prefix
is owned by `railway.releaseImagePrefix` in `.github/deployment-services.json`;
its value is `railway-private`, giving packages such as
`ghcr.io/OWNER/REPOSITORY/railway-private-api`. Publishing and recovery validate
against that same namespace.

Configure the GitHub Actions secrets `RAILWAY_REGISTRY_USERNAME` and
`RAILWAY_REGISTRY_PASSWORD` with durable pull-only access to these packages.
Preparation requires both before publishing. It verifies an authenticated pull
and checks that GitHub reports the package visibility as private before recording
the artifact. The short-lived publishing token is never used
as Railway's recovery credential. Keep the new packages private. Existing public
packages cannot become private; retire them only after private deployment is
verified and no active deployment or unresolved cutover references them.

The API image contains the original maintenance binary at
`/app/bin/scope-maintenance`. Ordinary deployments and recovery extract it from
the manifest's API digest and check its recorded SHA-256 before use. Recovery
does not depend on the retention period of GitHub Actions build artifacts.
Keep every release image and its unique tag while a cutover that references it
remains unresolved; registry cleanup must not remove those digests. Recovery
cannot rebuild or substitute a deleted image.
The public `VITE_POSTHOG_HOST` and `VITE_POSTHOG_PROJECT_TOKEN` repository variables
supply the same analytics build configuration that Railway previously supplied.

The release rehearsal uses the dedicated Railway `release-proof` environment
configured under `railway.staging` in the deployment manifest. Keep runner and
load experiments in separate environments so they cannot replace services during
the availability measurement. The GitHub environment remains `staging` for its
secrets and deployment protection rules.


## Daily releases

`scope-production-deploy.yml` runs daily at 9:00 AM America/Chicago, including
local daylight-saving changes. GitHub may start scheduled jobs late. Pushes to
`main` do not start this workflow. Pull requests run validation without deploying.

Both scheduled and manual releases pin the main head at trigger time. Later
commits wait for the next release. Runs serialize through the production
concurrency group. The default `changed` scope compares each component with its
last successful production revision, so unchanged components do not rebuild or
deploy. Select `all` manually when a full redeployment is needed.

To release the latest main head manually:

```sh
gh workflow run scope-production-deploy.yml --ref main
```

Prepared application artifacts pass through Railway `release-proof` before production.
There is no normal-release bypass. Interrupted cutover recovery reuses its pinned
artifacts without repeating the rehearsal so production can reopen. The prepared
release workflow requires a successful release-proof job from the source run.
Imported releases build the smoke tools without rebuilding application images.
Complete manifests initialize fixtures; partial manifests issue a fresh test login
without resetting the existing catalog or stopping unchanged backend services. Web-only
releases use the same gate; a CLI release included with application changes waits
for it too. CLI-only releases keep their build and distribution checks. The
prepared-release replay workflow deploys only application components present in
its validated manifest; CLI distribution remains a separate release lane.

Railway `staging` is reserved for experiments and is outside this chain. The
proof workflow calls its default target `release-proof`; its GitHub credential
environment and internal manifest slot still use the name `staging`.

This follows the scheduled/manual entry points, immutable revision, serialized
publishing, and unchanged-release skipping in [T3's release workflow](https://github.com/pingdotgg/t3code/blob/main/.github/workflows/release.yml).
