#!/usr/bin/env bash
set -euo pipefail

manifest_path="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
maintenance_binary="${SCOPE_MAINTENANCE_BINARY:-./target/release/scope-maintenance}"
evidence_path="${SCOPE_STAGING_EVIDENCE_PATH:-staging-deployments.json}"
# The candidate checkout owns deployment identity even when the workflow runs from main.
SCOPE_DEPLOYMENT_SOURCE_SHA="$(git rev-parse --verify HEAD)"
export SCOPE_DEPLOYMENT_SOURCE_SHA

if [[ -z "${RAILWAY_TOKEN:-}" || -n "${RAILWAY_API_TOKEN:-}" ]]; then
  echo "Staging deployment requires only a staging-scoped RAILWAY_TOKEN." >&2
  exit 1
fi

manifest_json="$(jq -c . "$manifest_path")"
project_id="$(jq -er '.railway.projectId' "$manifest_path")"
production_environment_id="$(jq -er '.environments.production.environmentId' "$manifest_path")"
staging_environment_id="$(jq -er '.environments.staging.environmentId' "$manifest_path")"
staging_api_replicas="$(jq -er '.environments.staging.apiReplicas' "$manifest_path")"
staging_cache_url="https://$(jq -er '.environments.staging.cacheDomain' "$manifest_path")"
staging_router_url="https://$(jq -er '.environments.staging.routerDomain' "$manifest_path")"
database_service="$(jq -er '.railway.databaseServiceId' "$manifest_path")"
cache_service="$(jq -er '.services.cache.id' "$manifest_path")"
worker_service="$(jq -er '.services["run-worker"].id' "$manifest_path")"
api_service="$(jq -er '.services.api.id' "$manifest_path")"
router_service="$(jq -er '.environments.staging.routerServiceId' "$manifest_path")"
media_service="$(jq -er '.services["media-api"].id' "$manifest_path")"
media_worker_service="$(jq -er '.services["media-worker"].id' "$manifest_path")"
if [[ "$staging_environment_id" == "$production_environment_id" ]]; then
  echo "Staging environment matches production." >&2
  exit 1
fi

railway_read() {
  node "$(dirname "${BASH_SOURCE[0]}")/railway-read.mjs" "$@"
}

railway_scope=(
  --project "$project_id"
  --environment "$staging_environment_id"
)
status_json="$(railway_read status "${railway_scope[@]}" --json)"
services_json="$(railway_read service list "${railway_scope[@]}" --json)"
SCOPE_DEPLOYMENT_MANIFEST_JSON="$manifest_json" \
  SCOPE_RAILWAY_STATUS_JSON="$status_json" \
  SCOPE_RAILWAY_SERVICES_JSON="$services_json" \
  node .github/scripts/verify-staging-target.mjs >/dev/null

railway variable set "${railway_scope[@]}" --service "$api_service" --skip-deploys \
  "SCOPE_GIT_PUBLIC_URL=$staging_router_url" >/dev/null
railway variable set "${railway_scope[@]}" --service "$router_service" --skip-deploys \
  "SCOPE_REPO_ROUTER_BACKEND=scope-api.railway.internal:8080" \
  "SCOPE_REPO_ROUTER_READ_REPLICAS=$staging_api_replicas" >/dev/null

api_variables="$(railway_read variable list "${railway_scope[@]}" --service "$api_service" --json)"
if ! jq -e --arg expected "$staging_cache_url" '.SCOPE_CACHE_URL == $expected' \
  <<< "$api_variables" >/dev/null; then
  echo "Staging API SCOPE_CACHE_URL does not match the reviewed staging cache domain." >&2
  exit 1
fi
if ! jq -e --arg expected "$staging_router_url" '.SCOPE_GIT_PUBLIC_URL == $expected' \
  <<< "$api_variables" >/dev/null; then
  echo "Staging API SCOPE_GIT_PUBLIC_URL does not match the reviewed staging router domain." >&2
  exit 1
fi

router_variables="$(railway_read variable list "${railway_scope[@]}" --service "$router_service" --json)"
if ! jq -e \
  --arg backend 'scope-api.railway.internal:8080' \
  --arg replicas "$staging_api_replicas" \
  '.SCOPE_REPO_ROUTER_BACKEND == $backend and .SCOPE_REPO_ROUTER_READ_REPLICAS == $replicas' \
  <<< "$router_variables" >/dev/null; then
  echo "Staging router variables do not match the reviewed API topology." >&2
  exit 1
fi

export RAILWAY_PROJECT_ID="$project_id"
export SCOPE_RAILWAY_ENVIRONMENT_ID="$staging_environment_id"
export RAILWAY_DEPLOY_MESSAGE="Staging $SCOPE_DEPLOYMENT_SOURCE_SHA"

assert_writer_state() {
  local expected_running="$1"
  local current_services
  current_services="$(railway_read service list "${railway_scope[@]}" --json)"
  SERVICES_JSON="$current_services" \
    API_SERVICE="$api_service" \
    CACHE_SERVICE="$cache_service" \
    WORKER_SERVICE="$worker_service" \
    MEDIA_SERVICE="$media_service" \
    MEDIA_WORKER_SERVICE="$media_worker_service" \
    EXPECTED_RUNNING="$expected_running" \
    node -e '
const services = JSON.parse(process.env.SERVICES_JSON || "[]");
const expected = Number(process.env.EXPECTED_RUNNING);
for (const id of [process.env.API_SERVICE, process.env.CACHE_SERVICE, process.env.WORKER_SERVICE,
  process.env.MEDIA_SERVICE, process.env.MEDIA_WORKER_SERVICE]) {
  const service = services.find((candidate) => candidate.id === id);
  if (!service) process.exit(1);
  const replicas = service.replicas || {};
  if (expected === 0 && (replicas.running || 0) === 0 && (replicas.crashed || 0) === 0) continue;
  if (expected === 1 && service.status === "SUCCESS" && (replicas.configured || 0) >= 1 &&
      (replicas.running || 0) >= 1 && (replicas.crashed || 0) === 0) continue;
  process.exit(1);
}
'
}

assert_staging_topology() {
  local current_services
  current_services="$(railway_read service list "${railway_scope[@]}" --json)"
  SCOPE_DEPLOYMENT_MANIFEST_JSON="$manifest_json" \
    SCOPE_RAILWAY_STATUS_JSON="$status_json" \
    SCOPE_RAILWAY_SERVICES_JSON="$current_services" \
    SCOPE_VERIFY_STAGING_TOPOLOGY=1 \
    node .github/scripts/verify-staging-target.mjs >/dev/null
}

# Application artifacts are imported from the release owner. Never build or reset data here.
: "${SCOPE_PREPARED_RELEASE_PATH:?Prepared release manifest is required}"
node --input-type=module - "$SCOPE_PREPARED_RELEASE_PATH" "$manifest_path" "$maintenance_binary" <<'NODE'
import { readFileSync } from 'node:fs';
import { validatePreparedRelease, validateMaintenanceArtifact } from './.github/scripts/railway-artifact.mjs';
const [path, manifestPath, binary] = process.argv.slice(2);
const prepared = JSON.parse(readFileSync(path));
const manifest = JSON.parse(readFileSync(manifestPath));
validatePreparedRelease(prepared, { sourceSha: process.env.SCOPE_DEPLOYMENT_SOURCE_SHA, services: manifest.services });
if (prepared.components.api) validateMaintenanceArtifact(prepared, readFileSync(binary));
NODE

if jq -e '.components.api' "$SCOPE_PREPARED_RELEASE_PATH" >/dev/null; then
  assert_writer_state 0
  database_variables="$(railway_read variable list "${railway_scope[@]}" --service "$database_service" --json)"
  export SCOPE_STAGING_DATABASE_PUBLIC_URL
  SCOPE_STAGING_DATABASE_PUBLIC_URL="$(jq -er '.DATABASE_PUBLIC_URL' <<< "$database_variables")"
  snapshot_backfill_dir="$(mktemp -d "${RUNNER_TEMP:-/tmp}/scope-staging-backfill.XXXXXX")"
  export SCOPE_STAGING_SNAPSHOT_BACKFILL_DIR="$snapshot_backfill_dir"
  run_maintenance() {
    # Railway supplies these variables to the runner-local command.
    # shellcheck disable=SC2016
    railway run "${railway_scope[@]}" --service "$api_service" --no-local -- \
      sh -c 'DATABASE_URL="$SCOPE_STAGING_DATABASE_PUBLIC_URL" SCOPE_DATA_DIR="$SCOPE_STAGING_SNAPSHOT_BACKFILL_DIR" exec "$@"' \
      scope-maintenance "$maintenance_binary" "$1"
  }
  run_maintenance plan >/dev/null
  run_maintenance validate-workflow-catalogs
  # Apply the candidate schema without running physical cleanup commands here.
  run_maintenance apply
  run_maintenance backfill-landing-files
  run_maintenance backfill-workflow-catalogs
  rm -rf -- "$snapshot_backfill_dir"
  unset SCOPE_STAGING_DATABASE_PUBLIC_URL SCOPE_STAGING_SNAPSHOT_BACKFILL_DIR
fi

evidence_lines="$(mktemp)"
trap 'rm -f "$evidence_lines"' EXIT
export SCOPE_DEPLOYMENT_EVIDENCE_PATH="$evidence_lines"
# The router's readiness requires API replica discovery. Staging starts with
# writers stopped, so restore the API before activating its router.
for component in cache run-worker media-api media-worker api git-router web; do
  jq -e --arg component "$component" '.components[$component]' "$SCOPE_PREPARED_RELEASE_PATH" >/dev/null || continue
  service="$(jq -er --arg component "$component" '.services[$component].id' "$manifest_path")"
  export SCOPE_DEPLOYMENT_COMPONENT="$component"
  case "$component" in
    media-worker)
      image="$(jq -er '.components["media-worker"].image' "$SCOPE_PREPARED_RELEASE_PATH")"
      node .github/scripts/deploy-railway-image.mjs "$service" "$image"
      ;;
    *)
      case "$component" in
        cache) root=cache-service ;;
        git-router) root=repo-router ;;
        run-worker) root=worker ;;
        media-api) root=media ;;
        *) root="$component" ;;
      esac
      bash .github/scripts/deploy-railway.sh "$service" "$root"
      ;;
  esac
done
assert_staging_topology
jq -s --slurpfile manifest "$manifest_path" --arg commit "$SCOPE_DEPLOYMENT_SOURCE_SHA" --arg environmentId "$staging_environment_id" \
  '{commit: $commit, environmentId: $environmentId, candidateDeployments: 1,
    deployments: map({service: $manifest[0].services[.component].id, deploymentId: .evidenceId, status: "SUCCESS"})}' \
  "$evidence_lines" > "$evidence_path"
