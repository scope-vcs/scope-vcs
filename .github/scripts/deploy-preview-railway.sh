#!/usr/bin/env bash
set -euo pipefail

manifest_path="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
maintenance_binary="${SCOPE_MAINTENANCE_BINARY:?SCOPE_MAINTENANCE_BINARY is required}"
prepared_release="${SCOPE_PREPARED_RELEASE_PATH:?SCOPE_PREPARED_RELEASE_PATH is required}"
preview_environment_id="${SCOPE_RAILWAY_PREVIEW_ENVIRONMENT_ID:?SCOPE_RAILWAY_PREVIEW_ENVIRONMENT_ID is required}"
source_sha="${SCOPE_DEPLOYMENT_SOURCE_SHA:?SCOPE_DEPLOYMENT_SOURCE_SHA is required}"
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || { echo 'A full source revision is required.' >&2; exit 2; }

if [[ -z "${RAILWAY_TOKEN:-}" || -n "${RAILWAY_API_TOKEN:-}" ]]; then
  echo "Preview deployment requires only a preview-scoped RAILWAY_TOKEN." >&2
  exit 1
fi

project_id="$(jq -er '.railway.projectId' "$manifest_path")"
production_environment_id="$(jq -er '.environments.production.environmentId' "$manifest_path")"
staging_environment_id="$(jq -er '.environments.staging.environmentId' "$manifest_path")"
if [[ ! "$preview_environment_id" =~ ^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$ ||
  "$preview_environment_id" == "$production_environment_id" ||
  "$preview_environment_id" == "$staging_environment_id" ]]; then
  echo "Preview environment must be an explicit ID distinct from production and staging." >&2
  exit 1
fi

node --input-type=module - "$prepared_release" "$manifest_path" "$maintenance_binary" <<'NODE'
import { readFileSync } from 'node:fs';
import { validatePreparedRelease, validateMaintenanceArtifact } from './.github/scripts/railway-artifact.mjs';
const [path, manifestPath, binary] = process.argv.slice(2);
const prepared = JSON.parse(readFileSync(path));
const manifest = JSON.parse(readFileSync(manifestPath));
validatePreparedRelease(prepared, {
  sourceSha: process.env.SCOPE_DEPLOYMENT_SOURCE_SHA,
  services: manifest.services,
  components: ['cache', 'media-api', 'run-worker', 'media-worker', 'api', 'git-router', 'web'],
});
validateMaintenanceArtifact(prepared, readFileSync(binary));
NODE

run_maintenance() {
  bash .github/scripts/railway-private-maintenance.sh "$preview_environment_id" "$1"
}
run_maintenance plan >/dev/null
run_maintenance validate-workflow-catalogs
run_maintenance apply
run_maintenance backfill-workflow-catalogs
run_maintenance verify

evidence_dir="$(mktemp -d)"
trap 'rm -rf "$evidence_dir"' EXIT
export RAILWAY_PROJECT_ID="$project_id"
export SCOPE_RAILWAY_ENVIRONMENT_ID="$preview_environment_id"
export RAILWAY_DEPLOY_MESSAGE="Preview $SCOPE_DEPLOYMENT_SOURCE_SHA"

for component in cache media-api run-worker media-worker api git-router web; do
  service="$(jq -er --arg component "$component" '.services[$component].id' "$manifest_path")"
  export SCOPE_DEPLOYMENT_COMPONENT="$component"
  export SCOPE_DEPLOYMENT_EVIDENCE_PATH="$evidence_dir/$component.jsonl"
  if [[ "$component" == media-worker ]]; then
    node .github/scripts/deploy-railway-image.mjs configure-registry "$service"
    node .github/scripts/deploy-railway-image.mjs "$service" \
      "$(jq -er '.components["media-worker"].image' "$prepared_release")"
  else
    bash .github/scripts/deploy-railway.sh "$service"
  fi
done
