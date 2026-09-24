# Production recovery sets

The daily GitHub workflow captures the database, repository objects, media objects,
and the exact data-encryption keys in one age-encrypted archive. It publishes that
archive and a completion marker into a private, versioned AWS recovery bucket.
Railway snapshot schedules remain a separate recovery mechanism.

## What is preserved

- A PostgreSQL custom-format dump and required object references from the same
  exported, read-only repeatable-read snapshot. Application writes remain available.
- Exact repository and media object keys and encrypted bytes, with an inventory
  SHA-256 for every object.
- `SCOPE_OBJECT_ENCRYPTION_KEY`, which decrypts repository objects and Git segments,
  and `SCOPE_MEDIA_ENCRYPTION_KEY`, which decrypts media chunks. Both must be present
  and pass actual data decryption checks. Credentials for accessing buckets are
  never archived. Grant signing keys are optional in manually supplied key bundles;
  the daily workflow captures the two data-encryption keys and does not claim to
  preserve every operational signing credential.
- Media reference inventory follows the existing cleanup tombstones: deleted
  attachments retain database metadata but do not require deleted object bytes.
  Missing media without a tombstone still fails verification.
- Plaintext checksums checked against the captured database for required content,
  Git segments, media chunks, and whole media manifests. Unknown encryption formats
  or missing key versions fail recovery verification.

The rebuildable cache bucket is excluded by default. Its database rows remain in
the dump, and the manifest records the exclusion and object count. Before starting
restored services against an empty cache bucket, apply `rebuild-cache.sql` only to
the isolated restored database. `restore.py --restore-database` does this after a
successful restore when the manifest excludes cache. Preserve historical workflow
cache reports; only the five live cache storage/index/cleanup tables are reset.
Set `SCOPE_RECOVERY_INCLUDE_CACHE=true` for a manual capture that includes cache.

The manifest's `source_sha` identifies the recovery-tool checkout, not an assertion
that GitHub main equals the deployed application release. It also records checksums
of the Python recovery modules and the database dump. Keep the release artifacts
corresponding to the database schema separately.

## Online consistency and limits

The collector lists source objects, takes the database snapshot and reference
inventory, copies with ETag `If-Match`, and lists source objects again. Any inventory
change, failed conditional read, missing referenced object, wrong encryption key,
or plaintext checksum mismatch fails that capture. It starts a fresh snapshot on
each retry, at most three times. No production write pause or database write lock
is introduced. Busy storage can prevent a backup; a failed set is never reported
as complete.

Hard caps are 10 GiB of source object bytes, 100,000 objects, and a 1 GiB database
dump. Operators may reduce the object caps through `SCOPE_RECOVERY_MAX_BYTES` and
`SCOPE_RECOVERY_MAX_OBJECTS`, but cannot raise them without reviewing the code.
Plaintext object-envelope verification is bounded to 256 MiB per object; Git
segments verify as bounded frames. The scratch filesystem must hold the unencrypted
capture and encrypted archive concurrently. Scratch directories are mode 0700,
files mode 0600, and cleaned on normal success/failure. Use an ephemeral runner.

The measured production inventory on 2026-09-15 was 4,202 repository objects /
653,801,217 bytes and 10 media objects /1,046,336 bytes. The excluded cache had
13 objects /4,952,654,077 bytes. A daily durable-data archive is approximately
625 MiB plus a 5 MB database dump before encryption overhead. At 42 retained daily
sets, S3 Standard base storage is roughly $0.63/month using the published US East
first-tier $0.023/GB-month price. Requests, source egress, GitHub minutes, monitoring,
and recovery downloads are additional. Recheck pricing and inventory before
changing retention. [S3 pricing](https://aws.amazon.com/s3/pricing/)

## Protection and identities

`storage.yaml` creates a retained private bucket with SSE-S3, versioning,
GOVERNANCE Object Lock for 35 days, lifecycle expiration after 42 days, and explicit
writer/reader restrictions. Governance retention can be bypassed by a separately
authorized administrator. This protects against the backup writer and ordinary
application credentials; it does not claim immunity from account administration.

The writer assumes `scope-recovery-writer` using GitHub OIDC. Trust binds the
immutable repository ID, main branch, production environment subject, and exact reusable workflow
`.github/workflows/recovery-execute.yml@refs/heads/main`. It may append encrypted
objects under `sets/*` and emit the recovery metric. It cannot read, delete, change
retention, or bypass governance. Uploads require `AES256` and checksums. The separate
reader role can list and retrieve immutable versions and has no write authority.
Neither role uses the runner permissions boundary, which does not grant recovery
operations. The reader directly trusts only the supplied `scope-operator` IAM user in this
account with MFA present. The metadata audit role receives no object access.
Same-account explicit user trust authorizes role assumption without expanding the
user's identity policy. [AWS role trust](https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_policies_elements_principal.html)

Age adds independent recipient encryption. Only the public recipient is stored in
GitHub. Preserve the private identity outside Railway, GitHub, and this AWS account
in the owner's recovery storage. Losing it loses access to the recovery archives.
Do not rotate or discard it until all archives encrypted to it have expired or
been re-encrypted and verified. [age documentation](https://github.com/FiloSottile/age)

## Configure daily GitHub capture

1. Deploy `storage.yaml` with the existing GitHub OIDC provider, trusted recovery
   reader user ARN (`arn:aws:iam::ACCOUNT:user/scope-operator`), and existing security alert SNS topic. The SNS topic policy
   must allow CloudWatch publication from the exact
   `scope-production-recovery-set-health` alarm ARN and this account.
2. Set repository variables `SCOPE_RECOVERY_WRITER_ROLE_ARN`,
   `SCOPE_RECOVERY_BUCKET`, and `SCOPE_RECOVERY_AGE_RECIPIENT` from reviewed outputs
   and the owner's public age recipient.
3. Reuse `RAILWAY_TOKEN` and the existing **production environment** secret
   `SCOPE_RAILWAY_SSH_PRIVATE_KEY`. The reusable capture job selects that environment;
   its explicit named secret declaration and caller mapping enable secret resolution.
   The production environment supplies the key; do not create a repository-scoped
   copy or inherit unrelated repository secrets. The maintenance service must expose its private
   `DATABASE_URL`, `SCOPE_BUCKET_*`, and `SCOPE_OBJECT_ENCRYPTION_KEY`; the media API
   owns its `SCOPE_MEDIA_BUCKET_*` and `SCOPE_MEDIA_ENCRYPTION_KEY`.
   The writer trust requires the matching `:environment:production` subject while
   independently enforcing `ref: refs/heads/main` and the exact reusable workflow.
   Apply this trust update before running a newly environment-bound workflow.
4. Run the `Recovery capture` workflow manually once and verify a completed archive
   and isolated restore before relying on its daily 07:17 UTC schedule.

The SSH collector checks the explicit project/environment/service IDs from the
deployment manifest. It fetches only approved runtime fields, keeps them in its
process or protected temporary files, and never writes them to `GITHUB_ENV`, step
outputs, logs, or workflow artifacts. PostgreSQL tools run inside the maintenance
service, using its private connection. Only the database dump/reference archive
crosses back to the ephemeral runner; the database connection credential does not.

`RecoverySetComplete` in `Scope/Security/Recovery`, with dimension `BucketName`, is
1 only after both the versioned encrypted archive and its versioned completion
marker are confirmed. Failure emits 0. Missing daily executions alarm after two
daily periods. The checker and capture are separate: a healthy Railway snapshot
monitor does not establish healthy object/key escrow. A stopped workflow or failed
OIDC authentication is detected through missing recovery metrics.

The systemd service/timer are optional for a separately managed trusted recovery
host. They are not the primary deployed schedule and must never depend on an
interactive human/root AWS session or an application dispatch credential.

## Isolated restore proof

Use the reader role to download the exact `complete.json` version and the archive
version it names. The writer intentionally cannot perform this read. Preserve the
completion marker's SHA-256 separately with the restore evidence.

```bash
python3 -m venv /tmp/scope-recovery-venv
/tmp/scope-recovery-venv/bin/pip install --require-hashes -r deploy/aws/recovery/requirements.txt
/tmp/scope-recovery-venv/bin/python deploy/aws/recovery/restore.py recovery.tar.age \
  --identity /secure/offline-recovery-identity \
  --expected-sha256 "$ARCHIVE_SHA256" \
  --destination /tmp/isolated-scope-recovery
```

Install the reviewed age release before running these commands. The workflow pins
age 1.3.2 and verifies the Linux AMD64 archive SHA-256. The verifier authenticates the
entire age stream before using recovered files, rejects unsafe archive members,
checks encrypted inventory and database checksums, and decrypts every required
object against snapshot metadata. Extracted bytes are still private data; keep the
directory isolated and remove it after the drill.

For an empty local PostgreSQL drill database named `scope_recovery_drill_*`, provide
its connection via `SCOPE_RECOVERY_DRILL_DATABASE_URL` and add `--restore-database`.
The command rejects nonlocal servers and nonempty databases. Alternatively restore
`database.dump` into a network-none PostgreSQL container, run the current reviewed
maintenance binary's schema verification, then apply the cache reset there when
needed. Production databases are never restore targets.

The archive keeps S3 object keys in the encrypted inventory and stores bytes under
hashed local filenames. To bring up an isolated application, upload verified files
to new isolated buckets under each inventory entry's original `key`, configure the
escrowed encryption keys, and start services only after database/schema/object
verification. Bucket identifiers and access credentials must be newly provisioned.
An archive/decryption proof is not itself a browser/CLI/Git application canary.

## Tests

```bash
python3 deploy/aws/recovery/storage.test.py
python3 -m unittest discover -s deploy/aws/recovery/tests -v
```

The age roundtrip test requires `age` and `age-keygen` on PATH. Tests include wrong
key, missing object, ETag churn, checksum corruption, size caps, Git frame recovery,
media spanning an 8 MiB chunk boundary, and immutable multipart publication. Keep
the completion receipts and actual restore results with operational evidence;
unit tests do not prove the scheduled production job has run.
