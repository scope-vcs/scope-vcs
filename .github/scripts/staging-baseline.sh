#!/usr/bin/env bash
set -euo pipefail
# This snapshot contains staging fixture metadata only, not an object-store backup.
# Restored fixtures must pass candidate backfills, browser reads, and Git clone before
# staging succeeds. Never accept a production database URL.
: "${SCOPE_PRODUCTION_MIGRATION_PLAN:?Production migration plan is required}"
: "${SCOPE_STAGING_BASELINE_DIR:?Private staging baseline directory is required}"
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
gh api "repos/$GITHUB_REPOSITORY" --jq '.private' | grep -qx true
key="$(node .github/scripts/staging-baseline.mjs "$SCOPE_PRODUCTION_MIGRATION_PLAN")"
if ! ("$SCOPE_MAINTENANCE_BINARY" plan > "$SCOPE_STAGING_BASELINE_DIR/current-plan.json" &&
  node .github/scripts/staging-baseline.mjs "$SCOPE_PRODUCTION_MIGRATION_PLAN" "$SCOPE_STAGING_BASELINE_DIR/current-plan.json" >/dev/null); then
  # The artifact is private, captured with writers stopped, and tied to this exact environment.
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
  (cd "$SCOPE_STAGING_BASELINE_DIR/restore" && sha256sum --check database.sha256 >/dev/null)
  # Recreate the staging schema in one transaction so candidate-only tables cannot survive.
  pg_restore --no-owner --no-privileges --exit-on-error \
    --file="$SCOPE_STAGING_BASELINE_DIR/restore/database.sql" "$SCOPE_STAGING_BASELINE_DIR/restore/database.dump"
  { printf '%s\n' 'DROP SCHEMA public CASCADE;'; cat "$SCOPE_STAGING_BASELINE_DIR/restore/database.sql"; } | \
    psql "$DATABASE_URL" -X --single-transaction -v ON_ERROR_STOP=1 >/dev/null
  "$SCOPE_MAINTENANCE_BINARY" plan > "$SCOPE_STAGING_BASELINE_DIR/current-plan.json"
  node .github/scripts/staging-baseline.mjs "$SCOPE_PRODUCTION_MIGRATION_PLAN" "$SCOPE_STAGING_BASELINE_DIR/current-plan.json" >/dev/null
fi
# Require representative preexisting data; candidate seeding would invalidate the upgrade test.
[[ "$(psql "$DATABASE_URL" -XAt -v ON_ERROR_STOP=1 -c 'SELECT count(*) > 0 FROM scope_repositories')" == t ]]
# No schema change needs no new baseline dump. Reconciliation above still runs.
if jq -e '.pending | length == 0' "$SCOPE_PRODUCTION_MIGRATION_PLAN" >/dev/null; then
  exit 0
fi
pg_dump --dbname="$DATABASE_URL" --schema=public --format=custom --no-owner --no-privileges --file="$SCOPE_STAGING_BASELINE_DIR/database.dump"
(cd "$SCOPE_STAGING_BASELINE_DIR" && sha256sum database.dump > database.sha256)
restore_safe="$(jq -r '[.pending[].name | select(. == "m0033_git_segment_streaming_v2" or . == "m0035_retired_git_storage_cutover")] | length == 0' "$SCOPE_PRODUCTION_MIGRATION_PLAN")"
jq -n --argjson safe "$restore_safe" --arg environment "$environment" --arg key "$key" \
  '{environmentId: $environment, ledgerHash: $key, metadataRestoreSafe: $safe}' > "$SCOPE_STAGING_BASELINE_DIR/baseline.json"
rm -rf "$SCOPE_STAGING_BASELINE_DIR/restore" "$SCOPE_STAGING_BASELINE_DIR/restore.zip"
echo "artifact_name=staging-baseline-$key" >> "$GITHUB_OUTPUT"
