# Request media operations

Request media has three Railway resources: the `scope-request-media` bucket, the public
`scope-media-api` gateway, and the private `scope-media-worker`. Their project-wide IDs and
environment domains live in `.github/deployment-services.json`. The reconciler compares
project resources with environment instances separately so adding production uses the same
recorded service and bucket IDs rather than creating duplicate project resources.

## Pre-alpha rollout policy

The owner approved temporary downtime for the initial rollout on 2026-09-08. Keep a private
recovery copy on the deployment operator's machine until production verification passes, then
delete that copy. This is a one-time cutover backup; no recurring backup schedule or retained
history is configured by this rollout.

Before cutover, close writes and capture the database, existing repository objects, service
configuration, and keys in that private copy. Verify the database can restore and the copied
objects match their inventory. Production has no existing media objects before this first
activation. Subsequent backups containing media must include the full backup unit below.

Use the passing staging capacity receipt and deployment checks to verify the release. The
pre-alpha rollout does not require zero downtime or a new monitoring service. Production
verification must cover service health, existing repository reads, Git access, and media
upload, processing, playback, and cleanup before deleting the recovery copy.

## Reconcile Railway

Use the Railway CLI version pinned in `.github/deployment-services.json`. Every command below
asserts the project and environment IDs from that manifest.

Use the actual Railway name in `environments.staging.environmentName` for the default rehearsal
target, `staging`. Its media gateway domain remains unset until the reconciler
creates and records that environment's resources. A domain from another environment cannot be
reused as its readiness endpoint.

Read-only plans:

```bash
node deploy/railway/reconcile-media.mjs plan --environment staging
node deploy/railway/reconcile-media.mjs plan --environment production \
  --worker-image 'ghcr.io/scope-vcs/scope-media-worker@sha256:<digest>'
```

The first production apply attaches the exact project-wide bucket, gateway, and worker IDs to
production. If sealed secrets are absent, it stops there and reports the manual actions without
applying service configuration or deploying the worker. Generate an Ed25519 signing pair and a
random 32-byte base64 encryption key offline. Save the encryption key to the chosen escrow
destination before sealing it; Railway cannot return a sealed value later. In the Railway UI:

- set and seal `scope-api.SCOPE_MEDIA_GRANT_PRIVATE_KEY` to the PKCS#8 private signing key;
- set `scope-media-api.SCOPE_MEDIA_GRANT_PUBLIC_KEY` to the matching SPKI public key;
- set and seal `scope-media-api.SCOPE_MEDIA_ENCRYPTION_KEY` and
  `scope-media-worker.SCOPE_MEDIA_ENCRYPTION_KEY` to the same encryption key.

The public signing key is configuration, not a secret. Keep the private signing key and
encryption key separate. Rotating the signing pair invalidates grants. Replacing the encryption
key without re-encrypting every stored chunk makes existing media unreadable.

After the recovery copy and sealed variables are complete, rerun apply and verification:

```bash
node deploy/railway/reconcile-media.mjs apply --environment production \
  --worker-image 'ghcr.io/scope-vcs/scope-media-worker@sha256:<digest>' \
  --write-manifest
node deploy/railway/reconcile-media.mjs verify --environment production \
  --worker-image 'ghcr.io/scope-vcs/scope-media-worker@sha256:<digest>'
```

Verification requires the recorded resource IDs and generated gateway domain, exact HTTPS web
origin, bucket references, one gateway and worker replica in the manifest region, gateway
`/readyz`, worker `/healthz`, no worker public domain, sealed secret metadata, and the reviewed
worker image pinned by digest.

## Staging proof

The `scope-media-worker` GHCR package remains private. The staging and production environments
must provide `RAILWAY_REGISTRY_USERNAME` and `RAILWAY_REGISTRY_PASSWORD`; the trusted deployment
owner stores those pull credentials on the Railway media worker service before activating the
digest-pinned image. Candidate code never receives the credentials.

The normal Release workflow activates the exact prepared candidate in staging and
runs browser, Git, and media smoke checks before production. It issues a temporary
staging-scoped token and revokes it when the job finishes. The media receipt records
the tested revision and deployments.

Use `deployment-tests.yml` with a prepared release run ID for repeated transition
tests. Capacity testing remains available through `dev/media-capacity.mjs` against
the retained staging environment. Staging and production have distinct physical
bucket instances and keys.

## Capacity proof

Use a valid video between 490 MB and 500 MiB, not random bytes, plus a valid photo. The staging
workflow creates this fixture and invokes the harness without exporting its private session. The harness
runs one large recording and four small uploads through three concurrent smoke flows while it
samples authenticated request-list latency. With a local gateway PID it also samples gateway
RSS. Each flow records processing time, initial-range playback time, cross-chunk seek time, full
download time, byte integrity, derivative reads, and cleanup.

For a local diagnostic run only, invoke the same harness with a short-lived test session:

```bash
SCOPE_MEDIA_SMOKE_TOKEN='<short-lived private smoke session>' \
node dev/media-capacity.mjs \
  --api 'https://scope-api-staging.up.railway.app' \
  --media-origin 'https://scope-media-api-staging.up.railway.app' \
  --repo dev/public-demo \
  --source-sha '<exact 40-character deployed commit>' \
  --large-video /secure/path/valid-500mb-recording.mp4 \
  --photo /secure/path/photo.png \
  --small-uploads 4 \
  --output .tmp/media-capacity/staging.json
```

Do not log or store the smoke token in the receipt. Retain the capacity receipt and its
`flow-*.json` files with the staging deployment evidence. Record the accepted numeric values for:

- API loaded p95 and maximum latency relative to baseline;
- peak gateway RSS and the Railway memory limit;
- large-video processing duration and processing-queue age;
- initial playback and cross-chunk seek duration;
- conversion failure rate, scratch-space high-water mark, cleanup backlog age, and upload bytes.

Set alerts only after the staging run supplies those values. Unmeasured values remain explicit
verification limits; do not turn local measurements into claimed staging results.

## Lifecycle and cleanup

- A prepared but unfinished upload expires 24 hours after preparation.
- A completed attachment with no binding expires seven days after upload completion. Removing
  its final markdown binding starts a new seven-day window.
- Adding a binding clears the unbound expiry. Bound media is retained when a request merges or
  closes; request terminal state is not a cleanup trigger.
- Repository deletion, incomplete-upload expiry, and unbound-draft expiry create cleanup jobs.
  The worker deletes every inventoried object under a renewable lease, records a tombstone, and
  retries failures. Late-write grace and periodic tombstone reconciliation prevent a write that
  finishes after lease loss from permanently resurrecting bytes.

Alert on oldest processing-queue age, expired or repeatedly reclaimed processing leases,
conversion failures by code, scratch usage, oldest cleanup job, cleanup retries, and gateway
upload/read bytes. Worker `/healthz` already fails when codecs, schema, storage, or either poll
loop is stale; gateway `/readyz` fails when its schema or object store is unavailable. These
health checks are deployment fences, not substitutes for backlog and capacity alerts.

## Backup unit

A usable media backup is one recoverable unit containing all of the following:

1. A consistent Postgres backup of the full Scope database. Media manifests, chunk inventory,
   bindings, authorization context, processing jobs, cleanup jobs, and tombstones span the
   `scope_request_media_*` tables and related request/repository records.
2. Every encrypted object referenced by that database snapshot from the environment's
   `scope-request-media` bucket. Preserve object keys and bytes exactly.
3. The exact `SCOPE_MEDIA_ENCRYPTION_KEY` version used for those objects, held in independent
   secret escrow. Without it, the encrypted chunks are intentionally unrecoverable.

Also back up normal service configuration and the media grant signing keys for service recovery,
but do not confuse the signing key with the data-encryption key.

For a consistent backup, fence API, gateway, and worker writes, wait for active upload,
processing, and cleanup leases to drain, take the database snapshot, copy the bucket, and then
release the fence. Write an immutable backup manifest containing the environment and source SHA,
database snapshot ID/time, bucket ID, object count and total bytes, inventory checksum, escrowed
key version ID, and backup-tool result. Fail the backup if any field or component is missing.

## Restore drill

Restore into an isolated environment with no public gateway domain:

1. Restore the database snapshot and encrypted objects using their original object keys.
2. Inject the exact escrowed encryption key into both gateway and worker. Keep the worker at zero
   replicas until the database and object inventory are in place.
3. Compare restored object count, bytes, and inventory checksum with the backup manifest. Treat a
   missing referenced object as restore failure; extra objects may be cleanup candidates but must
   be investigated.
4. Start one worker and one private gateway, verify `/healthz` and `/readyz`, then read sampled
   bound originals and derivatives. Include a range crossing an 8 MiB chunk boundary and compare
   plaintext SHA-256 with metadata.
5. Enable a temporary authenticated endpoint only for the application smoke, confirm anonymous
   access is denied, then remove the endpoint and the isolated environment.

The storage-level recovery test proves that serialized manifest metadata plus encrypted chunks
larger than one 8 MiB chunk restore into a fresh store with the correct key, and that a wrong key
fails integrity validation:

```bash
cargo test --locked -p scope-media-storage --test recovery
```

That test is evidence for the storage format and key dependency. It does not prove an external
backup destination, retention policy, database snapshot, or Railway restore procedure.
