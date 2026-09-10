#!/usr/bin/env bash
set -euo pipefail
umask 077
# This snapshot contains staging fixture metadata only, not an object-store backup.
# Restored fixtures must pass candidate backfills, browser reads, and Git clone before
# staging succeeds. Never accept a production database URL.
: "${SCOPE_PRODUCTION_MIGRATION_PLAN:?Production migration plan is required}"
: "${SCOPE_STAGING_BASELINE_DIR:?Staging baseline directory is required}"
: "${SCOPE_MAINTENANCE_BINARY:?Pinned maintenance binary is required}"
[[ -n "${RAILWAY_TOKEN:-}" && -z "${RAILWAY_API_TOKEN:-}" ]]
manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
project="$(jq -er '.railway.projectId' "$manifest")"
environment="$(jq -er '.environments.staging.environmentId' "$manifest")"
database="$(jq -er '.railway.databaseServiceId' "$manifest")"
[[ "$environment" != "$(jq -er '.environments.production.environmentId' "$manifest")" ]]
scope=(--project "$project" --environment "$environment")
SCOPE_DEPLOYMENT_MANIFEST_JSON="$(cat "$manifest")" \
  SCOPE_RAILWAY_STATUS_JSON="$(node .github/scripts/railway-read.mjs status "${scope[@]}" --json)" \
  SCOPE_RAILWAY_SERVICES_JSON="$(node .github/scripts/railway-read.mjs service list "${scope[@]}" --json)" \
  node .github/scripts/verify-staging-target.mjs >/dev/null
# Verify that metadata writers have relinquished the database fence before snapshot/restore.
variables="$(node .github/scripts/railway-read.mjs variable list "${scope[@]}" --service "$database" --json)"
export DATABASE_URL
DATABASE_URL="$(jq -er '.DATABASE_PUBLIC_URL | strings | select(length > 0)' <<< "$variables")"
node --input-type=module - "$SCOPE_PREPARED_RELEASE_PATH" "$SCOPE_MAINTENANCE_BINARY" <<'NODE'
import { readFileSync } from 'node:fs';
import { validateMaintenanceArtifact } from './.github/scripts/railway-artifact.mjs';
validateMaintenanceArtifact(JSON.parse(readFileSync(process.argv[2])), readFileSync(process.argv[3]));
NODE
"$SCOPE_MAINTENANCE_BINARY" fence >/dev/null
mkdir -p "$SCOPE_STAGING_BASELINE_DIR"
chmod 0700 "$SCOPE_STAGING_BASELINE_DIR"
# Clear plaintext and extracted artifacts even if authentication or restoration fails.
cleanup() {
  rm -f "$SCOPE_STAGING_BASELINE_DIR/database.dump"
  rm -rf "$SCOPE_STAGING_BASELINE_DIR/restore" "$SCOPE_STAGING_BASELINE_DIR/restore.zip"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
key="$(node .github/scripts/staging-baseline.mjs "$SCOPE_PRODUCTION_MIGRATION_PLAN")"
if ! ("$SCOPE_MAINTENANCE_BINARY" plan > "$SCOPE_STAGING_BASELINE_DIR/current-plan.json" &&
  node .github/scripts/staging-baseline.mjs "$SCOPE_PRODUCTION_MIGRATION_PLAN" "$SCOPE_STAGING_BASELINE_DIR/current-plan.json" >/dev/null); then
  # Retained artifacts contain authenticated ciphertext, captured with writers stopped.
  : "${SCOPE_STAGING_BASELINE_KEY:?Staging baseline encryption key is required to restore a snapshot}"
  : "${GITHUB_REPOSITORY:?}"
  artifact_id="$(gh api --paginate "repos/$GITHUB_REPOSITORY/actions/artifacts?name=staging-baseline-$key&per_page=100" \
    --jq '.artifacts[] | select(.expired == false and .workflow_run.head_branch == "main") | .id' | sed -n '1p')"
  [[ -n "$artifact_id" ]] || { echo 'No retained staging baseline matches production; explicit baseline provisioning is required.' >&2; exit 1; }
  run_id="$(gh api "repos/$GITHUB_REPOSITORY/actions/artifacts/$artifact_id" --jq '.workflow_run.id')"
  gh api "repos/$GITHUB_REPOSITORY/actions/runs/$run_id" \
    --jq '.path == ".github/workflows/release.yml" and .head_branch == "main" and (.event == "schedule" or .event == "workflow_dispatch")' | grep -qx true
  gh api "repos/$GITHUB_REPOSITORY/actions/artifacts/$artifact_id/zip" > "$SCOPE_STAGING_BASELINE_DIR/restore.zip"
  unzip -q "$SCOPE_STAGING_BASELINE_DIR/restore.zip" -d "$SCOPE_STAGING_BASELINE_DIR/restore"
  jq -e --arg environment "$environment" --arg key "$key" \
    '.environmentId == $environment and .ledgerHash == $key and .metadataRestoreSafe == true' "$SCOPE_STAGING_BASELINE_DIR/restore/baseline.json" >/dev/null
  (cd "$SCOPE_STAGING_BASELINE_DIR/restore" &&
    [[ "$(wc -l < database.sha256)" == 1 ]] &&
    grep -Eq '^[0-9a-f]{64}  database\.dump\.enc$' database.sha256 &&
    sha256sum --check database.sha256 >/dev/null)
  node .github/scripts/staging-baseline-crypto.mjs decrypt \
    "$SCOPE_STAGING_BASELINE_DIR/restore/database.dump.enc" \
    "$SCOPE_STAGING_BASELINE_DIR/restore/database.dump" \
    "$SCOPE_STAGING_BASELINE_DIR/restore/baseline.json"
  # Recreate the staging schema in one transaction so candidate-only tables cannot survive.
  pg_restore --no-owner --no-privileges --exit-on-error \
    --file="$SCOPE_STAGING_BASELINE_DIR/restore/database.sql" "$SCOPE_STAGING_BASELINE_DIR/restore/database.dump"
  { printf '%s\n' 'DROP SCHEMA public CASCADE;'; cat "$SCOPE_STAGING_BASELINE_DIR/restore/database.sql"; } | \
    psql "$DATABASE_URL" -X --single-transaction -v ON_ERROR_STOP=1 >/dev/null
  "$SCOPE_MAINTENANCE_BINARY" plan > "$SCOPE_STAGING_BASELINE_DIR/current-plan.json"
  node .github/scripts/staging-baseline.mjs "$SCOPE_PRODUCTION_MIGRATION_PLAN" "$SCOPE_STAGING_BASELINE_DIR/current-plan.json" >/dev/null
fi
# A matching ledger does not prove matching schema. Reject drift before retaining
# or migrating this staging baseline, including snapshots restored above.
"$SCOPE_MAINTENANCE_BINARY" preflight >/dev/null
# Require representative preexisting data; candidate seeding would invalidate the upgrade test.
[[ "$(psql "$DATABASE_URL" -XAt -v ON_ERROR_STOP=1 -c 'SELECT count(*) > 0 FROM scope_repositories')" == t ]]
# No schema change needs no new baseline dump. Reconciliation above still runs.
if jq -e '.pending | length == 0' "$SCOPE_PRODUCTION_MIGRATION_PLAN" >/dev/null; then
  exit 0
fi
: "${SCOPE_STAGING_BASELINE_KEY:?Staging baseline encryption key is required to retain a snapshot}"
pg_dump --dbname="$DATABASE_URL" --schema=public --format=custom --no-owner --no-privileges --file="$SCOPE_STAGING_BASELINE_DIR/database.dump"
# Manifest retirement queues object deletion when the candidate worker starts.
# A database-only snapshot cannot restore those objects after the staging trial.
restore_safe="$(jq -r '[.pending[].name | select(. == "m0043_retire_git_manifests")] | length == 0' "$SCOPE_PRODUCTION_MIGRATION_PLAN")"
jq -n --argjson safe "$restore_safe" --arg environment "$environment" --arg key "$key" \
  '{environmentId: $environment, ledgerHash: $key, metadataRestoreSafe: $safe}' > "$SCOPE_STAGING_BASELINE_DIR/baseline.json"
node .github/scripts/staging-baseline-crypto.mjs encrypt \
  "$SCOPE_STAGING_BASELINE_DIR/database.dump" \
  "$SCOPE_STAGING_BASELINE_DIR/database.dump.enc" \
  "$SCOPE_STAGING_BASELINE_DIR/baseline.json"
(cd "$SCOPE_STAGING_BASELINE_DIR" && sha256sum database.dump.enc > database.sha256)
echo "artifact_name=staging-baseline-$key" >> "$GITHUB_OUTPUT"
