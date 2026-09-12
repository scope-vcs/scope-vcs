# Releases and maintenance recovery

`release.yml` is the production entry point. It polls at minutes 8 and 38 and
releases once per Chicago calendar day after 9 AM. Failed runs remain eligible
for the next poll. Manual dispatch bypasses the daily timing check:

```sh
gh workflow run release.yml --ref main
```

Both paths pin the main revision before preparation and share the production
concurrency lock. Later commits wait for another release. Component receipts
skip unchanged work. `ci.yml` validates pull requests without deploying; it and
Release call the same reusable validation workflow.

Preparation publishes private GHCR images and records their immutable digests.
Staging activates the candidate once and checks browser, Git, and media behavior.
Production uses those exact images. Optional `deployment-tests.yml` runs repeated
transition tests using a prepared release, outside the normal release path.

## Environments and names

`.github/deployment-services.json` owns the two deployment targets under
`environments.production` and `environments.staging`. The staging environment ID
is the former release-proof environment. Its existing generated domains stay
valid after the environment is renamed. GitHub uses the credential environment
`staging` for staging work.

Component keys describe their roles: `api`, `run-worker`, `cache`, `git-router`,
`media-api`, `media-worker`, `web`, and `cli-downloads`. `checks-image` and
`cli-distribution` track the other published artifacts. Runtime executable names
and existing image repositories are independent of these orchestration keys.

## Migration policy

Every pending migration requires maintenance. Runtime startup checks the exact
migration ledger and refuses a mismatch; it never applies migrations. Development
and test setup invoke migration application explicitly. A production migration
requires a complete prepared application set so schema participants move together.

The manifest's `releasePolicy` owns these defaults:

- Maintenance is enabled automatically for pending migrations.
- Thirty minutes of maintenance produces a warning, not a failed healthy release.
- Writer drain and migration lock acquisition each allow 120 seconds.
- Each migration statement allows 3,600 seconds.
- Production observation continues for 60 seconds after activation.

Operation timeouts still fail their operation. The maintenance warning is separate
from those limits, recovery, and the final health result.

Before closure, the workflow records a durable `production/maintenance` deployment
with the exact prepared manifest, baseline ledger, previous deployments, and
maintenance service configuration. Phase statuses record intent before mutations.
Public services temporarily run the prepared API image's `scope-maintenance serve`
command. It serves an HTML maintenance page to browsers and a structured 503 to
API, Git, and media clients, with `Retry-After`. It needs no database connection.
Only its `/readyz` endpoint returns success.

The cutover stops metadata writers, acquires the database writer fence, applies
migrations and required backfills, and verifies the exact ledger. It then activates
prepared services in dependency order. Public services reopen as their dependencies
become ready; API activation follows its backend dependencies, and web opens last.
The journal completes only after the coordinated activation succeeds.

## Recovery

Dispatch Release again. It automatically selects the sole unresolved cutover,
including its original SHA and image digests. It refuses ambiguous multiple open
cutovers. No copied deployment IDs or replacement builds are required.

Recovery closes writers again before reading the ledger. An exact candidate ledger
resumes forward activation. An unchanged baseline resumes the pinned migration.
An unknown or unreadable ledger leaves writers closed.

Once a maintenance gate starts replacing deployments, recovery proceeds forward:
Railway may already have removed the previous deployment IDs. A committed migration
or an uncertain external backfill also prevents old-binary restoration. Runtime
schema verification and the exclusive database fence remain the final write boundary.

To deploy an already prepared, validated release through the same workflow:

```sh
gh workflow run release.yml --ref main -f source_run_id=RUN_ID
```

Replay validates the source run, staging evidence, source revision, and immutable
manifest. Interrupted cutover recovery uses the durable record directly so an
unrelated staging failure cannot prevent production recovery.

Read-only inspection uses the pinned maintenance binary:

```text
scope-maintenance plan
scope-maintenance verify
node .github/scripts/release-cutover-journal.mjs cutover-read --id DEPLOYMENT_ID --source-sha FULL_SHA
```

`plan` emits `exact`, ordered `applied` migration names, and pending names. `verify`
succeeds only for an exact ledger.

## Resume staging after a smoke failure

When a release completed `Deploy candidate once` but failed a later smoke check,
run `Release` with its original `source_run_id` and `resume_staging=true`.
The resume keeps the original application images and maintenance binary. It
requires successful original validation and image preparation on main, the
complete staging deployment receipt, matching original and latest Railway image
digests, and an exact candidate migration ledger with nothing pending.

Cleanup may have removed the staging deployments after the smoke failure. Resume
reactivates those pinned images with writers closed first, without restoring the
pre-migration database, applying migrations, or repeating backfills. Browser smoke
checks come from the current trusted workflow revision so a corrected smoke test
does not require rebuilding the application images. Production stays blocked
until the resumed browser, Git, and media smoke checks succeed. An unresolved
production cutover must recover before a staging resume can run.

## Staging baseline

Staging records an encrypted database snapshot keyed to the production applied ledger
before testing a migration candidate. The Actions artifact is retained for seven
days and is safe to retain in this public repository: it contains only
`database.dump.enc`, its ciphertext checksum, and nonsecret `baseline.json` metadata.
AES-256-GCM authenticates both the dump and the environment, ledger, and restore
policy metadata before restoration can modify the database. Plaintext temporary
files stay in a private directory and are removed when the baseline step exits.

The GitHub `staging` environment owns `SCOPE_STAGING_BASELINE_KEY`, a random
32-byte key encoded as exactly 64 hexadecimal characters. Only the baseline step
receives it. Keep that key available for the full snapshot retention period;
rotating it requires capturing a new baseline before an older snapshot is needed.
Plaintext archives and snapshots encrypted under another key are rejected.
Matching ledgers with no pending migrations require neither the key nor an archive
upload. If a failed candidate leaves staging ahead, the next attempt restores a
matching retained baseline while writers are fenced. Unknown baselines and snapshots
that cross known external storage transformations fail explicitly. Required
backfills run in staging as well as production; physical object cleanup does not
run as part of candidate testing.

A database snapshot does not back up object storage. Preserve the staging object
stores and keys across these tests. Unseeded or incompatible staging needs an
explicit baseline reset before the normal release path can proceed.

## Release image storage

Release images use private GHCR packages addressed by digest. The prefix
`railway.releaseImagePrefix` produces packages such as
`ghcr.io/OWNER/REPOSITORY/railway-private-api`. Preparation verifies package privacy
and an authenticated pull before recording artifacts.

GitHub Actions secrets `RAILWAY_REGISTRY_USERNAME` and `RAILWAY_REGISTRY_PASSWORD`
provide durable pull-only access. The short-lived publishing token is not a
recovery credential. The API image contains `/app/bin/scope-maintenance`; extraction
checks its SHA-256 against the prepared manifest without starting the container.

Retain all image digests and their unique tags while a referencing cutover remains
unresolved. Recovery cannot rebuild or substitute a deleted image. Public analytics
build configuration comes from the repository variables `VITE_POSTHOG_HOST` and
`VITE_POSTHOG_PROJECT_TOKEN`.
