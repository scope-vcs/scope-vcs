# Private PostgreSQL credential cutover

Run staging first. Complete its restore and application canaries before repeating
this sequence for production. `prepare-cutover.mjs` only prepares files; it does
not connect to PostgreSQL or Railway and does not change any service.

## Prepare the bundle

Use the verified maintenance image and registered operator SSH key. Keep the
maintenance service on its bootstrap administrator connection until all six new
logins and runtime service variables have been verified.

```bash
set -euo pipefail
umask 077
cutover_environment=staging
manifest=.github/deployment-services.json
environment_id="$(jq -er --arg name "$cutover_environment" '.environments[$name].environmentId' "$manifest")"
project_id="$(jq -er '.railway.projectId' "$manifest")"
bundle_dir="/tmp/scope-db-cutover-$cutover_environment-$(date -u +%Y%m%dT%H%M%SZ)"
export SCOPE_RAILWAY_SSH_IDENTITY_FILE=/home/adam-blumoff/.ssh/scope-maintenance-ci-20260915
source .github/scripts/railway-private-command.sh
railway_private_command "$environment_id" sh -ceu 'printf "%s" "$DATABASE_URL"' |
  node deploy/postgres/prepare-cutover.mjs "$cutover_environment" "$bundle_dir"
jq . "$bundle_dir/plan.json"
```

The URL travels directly between processes, never through terminal output or
command arguments. The new directory is 0700 and every file is 0600. It contains
one atomic role/ownership/password bootstrap script, six connection checks and
six Railway variable payloads. Only `plan.json` and the summary are safe to print.
Do not print, upload, commit, or enable shell tracing around the other files.

Every generated connection requires TLS. Missing or weaker `sslmode` values
become `require`; explicit `verify-ca` and `verify-full` remain intact.
Passwords are independent 256-bit random values. SQL contains PostgreSQL SCRAM
verifiers for the new passwords. Bootstrap scripts pass connection credentials
through libpq environment variables and SQL through stdin. The generator refuses
public database hosts, ambiguous targets, and existing output directories.

## Close old writers and bootstrap

Record the exact active deployments and their immutable images for API, run
worker, cache, media API, and media worker. Coordinate the existing public
maintenance gates for production. Stop each exact writer deployment using the
existing release controls. Do not change the database service or stop the private
maintenance service.

While maintenance still connects as the administrator, terminate only the
sessions holding Scope's metadata writer fence. This handles the old runtimes
that connected as the PostgreSQL superuser. `scope_migrator` cannot terminate
superuser sessions.

```bash
railway_private_command "$environment_id" /app/bin/scope-maintenance drain-writers < /dev/null
railway_private_command "$environment_id" /app/bin/scope-maintenance fence < /dev/null
railway_private_command "$environment_id" sh -s < "$bundle_dir/bootstrap.sh"
for component in api run-worker cache media-api media-worker maintenance; do
  railway_private_command "$environment_id" sh -s < "$bundle_dir/$component.verify.sh"
done
```

The bootstrap transaction either installs all roles, grants and new password
verifiers or rolls back. Keep writers closed if any command fails. Before
bootstrap, confirm a current recovery snapshot exists and inventory live tables
against the reviewed role policy. An unexpected table fails the transaction.

## Select runtime credentials

Apply each fixed, environment-scoped variable payload through stdin. The
GraphQL input was checked against Railway's live schema. It changes only
`DATABASE_URL`, preserves other variables, and sets `skipDeploys: true`.

```bash
for component in api run-worker cache media-api media-worker; do
  railway api 'mutation DatabaseCredential($input:VariableCollectionUpsertInput!){variableCollectionUpsert(input:$input)}' \
    --variables @- --compact < "$bundle_dir/$component.variables.json" |
    jq -e '(.errors // [] | length) == 0 and .data.variableCollectionUpsert == true' >/dev/null
done
```

Create fresh deployments of the recorded immutable images using the existing
release owner. A restart of an old deployment may retain its old environment and
must not count as credential activation. Restore gate configurations before
creating the new API/media deployments. Verify all five services become healthy
and repeat their role-specific connection checks.

Update maintenance last, then deploy the same verified maintenance image again
to load its migration login:

```bash
railway api 'mutation DatabaseCredential($input:VariableCollectionUpsertInput!){variableCollectionUpsert(input:$input)}' \
  --variables @- --compact < "$bundle_dir/maintenance.variables.json" |
  jq -e '(.errors // [] | length) == 0 and .data.variableCollectionUpsert == true' >/dev/null
# Use deploy-maintenance-runtime.mjs with the retained image receipt, then:
bash .github/scripts/verify-maintenance-runtime.sh "$cutover_environment" maintenance-runtime.json
```

Keep the administrator recovery path in the database service. Do not put its
credentials back into application settings if a canary fails. Correct the
reviewed grant and repeat the failed canary with writers closed. Preserve the
private bundle until the cutover and recovery checks finish, then remove it.

## Required staging proof

With staging writers stopped, the following helper streams connection fields and
SQL directly over SSH. Passwords never appear in a process argument, and dump SQL
never passes through a shell evaluator.

```bash
[[ "$cutover_environment" == staging ]]
private_db_tool() {
  { cat "$bundle_dir/maintenance.connection"; cat; } |
    railway_private_command "$environment_id" sh -ceu '
      IFS= read -r PGHOST
      IFS= read -r PGPORT
      IFS= read -r PGDATABASE
      IFS= read -r PGUSER
      IFS= read -r PGPASSWORD
      IFS= read -r PGSSLMODE
      export PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD PGSSLMODE
      export PGCONNECT_TIMEOUT=10
      exec "$@"
    ' scope-db-tool "$@"
}
private_db_tool pg_dump --format=custom --no-owner --no-privileges \
  < /dev/null > "$bundle_dir/staging.dump"
railway_private_command "$environment_id" /app/bin/scope-maintenance plan \
  < /dev/null > "$bundle_dir/staging-plan.json"
ledger_hash="$(node .github/scripts/staging-baseline.mjs "$bundle_dir/staging-plan.json")"
jq -n --arg environmentId "$environment_id" --arg ledgerHash "$ledger_hash" \
  '{environmentId:$environmentId,ledgerHash:$ledgerHash,metadataRestoreSafe:true}' \
  > "$bundle_dir/baseline.json"
node .github/scripts/staging-baseline-crypto.mjs encrypt \
  "$bundle_dir/staging.dump" "$bundle_dir/staging.dump.enc" "$bundle_dir/baseline.json"
rm -- "$bundle_dir/staging.dump"
node .github/scripts/staging-baseline-crypto.mjs decrypt \
  "$bundle_dir/staging.dump.enc" "$bundle_dir/staging.dump" "$bundle_dir/baseline.json"
docker run --rm -i --entrypoint pg_restore \
  postgres:18.6@sha256:4ef4dbc939d61acea57712655ddb4b4ab27419c913f94cca0cd57cb3ea3c2280 \
  --no-owner --no-privileges --exit-on-error \
  < "$bundle_dir/staging.dump" > "$bundle_dir/staging.sql"
{ printf '%s\n' 'DROP SCHEMA public CASCADE;' 'CREATE SCHEMA public;'; cat "$bundle_dir/staging.sql"; } |
  private_db_tool psql -X -q --single-transaction -v ON_ERROR_STOP=1 >/dev/null
node deploy/postgres/runtime-roles.mjs --grants-only |
  private_db_tool psql -X -q -v ON_ERROR_STOP=1 >/dev/null
for component in api run-worker cache media-api media-worker maintenance; do
  railway_private_command "$environment_id" sh -s < "$bundle_dir/$component.verify.sh"
done
rm -- "$bundle_dir/staging.dump" "$bundle_dir/staging.sql"
```

The existing `SCOPE_STAGING_BASELINE_KEY` must be supplied through the process
environment, never printed or placed in a command argument. Preserve it with the
encrypted snapshot for recovery. The dump includes `pg_trgm` because it covers the
complete dedicated database. The decrypt step verifies its authentication tag
before restoration. Reapply grants before reopening writers. Old public-only
archives cannot establish this proof; regenerate them. The normal release
workflow retains a new baseline when a real migration is pending.

Run a complete staging release and browser/CLI sign-in, Git clone/push, request
mutations, media upload/read/processing/delete, cache restore/upload/GC and run
launch/logging/cancel/retry checks. Check worker compaction and content cleanup.
Database ACL checks alone do not prove those application transactions.

## Production and proxy removal

Repeat the credential cutover for production after staging passes. Run production
preflight, readiness and application canaries through the new logins. Confirm the
new deployments use their per-service usernames and only the maintenance service
uses `scope_migrator`. Confirm old writer sessions no longer hold the database
fence. Preserve the administrator credential only for database administration.

Only then read the exact database TCP proxy IDs and remove those proxies with
`mutation RemoveDatabaseProxy($id:String!){tcpProxyDelete(id:$id)}`. Check every
response is true. Do not delete domains or proxies for application services.
Verify the previous external database host/port refuses a connection, then repeat
private migration preflight, staging restore access and service readiness. Record
both the failed external connection and successful private checks as evidence.
