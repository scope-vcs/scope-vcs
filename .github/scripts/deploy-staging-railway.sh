#!/usr/bin/env bash
set -euo pipefail

action="${1:?usage: deploy-staging-railway.sh <prepare|finish> <upload-roots...>}"
shift
manifest_path="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
maintenance_binary="${SCOPE_MAINTENANCE_BINARY:-./target/release/scope-maintenance}"
seed_binary="${SCOPE_SMOKE_SEED_BINARY:-./target/release/scope-smoke-seed}"
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
production_environment_id="$(jq -er '.railway.environmentId' "$manifest_path")"
staging_environment_id="$(jq -er '.railway.staging.environmentId' "$manifest_path")"
staging_environment_name="$(jq -er '.railway.staging.environmentName' "$manifest_path")"
staging_api_replicas="$(jq -er '.railway.staging.apiReplicas' "$manifest_path")"
staging_cache_url="https://$(jq -er '.railway.staging.cacheDomain' "$manifest_path")"
staging_router_url="https://$(jq -er '.railway.staging.routerDomain' "$manifest_path")"
database_service="$(jq -er '.railway.databaseServiceId' "$manifest_path")"
cache_service="$(jq -er '.services.cache.id' "$manifest_path")"
worker_service="$(jq -er '.services.worker.id' "$manifest_path")"
api_service="$(jq -er '.services.api.id' "$manifest_path")"
router_service="$(jq -er '.railway.staging.routerServiceId' "$manifest_path")"
media_service="$(jq -er '.services.media.id' "$manifest_path")"
media_worker_service="$(jq -er '.services.mediaWorker.id' "$manifest_path")"
web_service="$(jq -er '.services.web.id' "$manifest_path")"
media_worker_image="${SCOPE_MEDIA_WORKER_IMAGE:-}"
if [[ ! "$media_worker_image" =~ ^ghcr\.io/scope-vcs/scope-media-worker@sha256:[0-9a-f]{64}$ ]]; then
  echo "SCOPE_MEDIA_WORKER_IMAGE must pin the staging worker image by digest." >&2
  exit 2
fi

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

record_deployment() {
  local service="$1"
  railway_read deployment list "${railway_scope[@]}" --service "$service" --limit 1 --json |
    jq -ec --arg service "$service" '
      first | select(.status == "SUCCESS") |
      {service: $service, deploymentId: .id, status: .status}
    '
}

case "$action" in
  prepare)
    if [[ "$#" -ne 5 || ! -x "$maintenance_binary" || ! -x "$seed_binary" ]]; then
      echo "usage: deploy-staging-railway.sh prepare <cache-root> <worker-root> <api-root> <router-root> <media-root>" >&2
      exit 2
    fi
    : "${SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH:?SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH is required}"
    assert_writer_state 0
    # The remote shell expands the Railway-provided database URL.
    # shellcheck disable=SC2016
    railway run "${railway_scope[@]}" --service "$database_service" --no-local -- \
      sh -c 'DATABASE_URL="$DATABASE_PUBLIC_URL" exec "$@"' \
      scope-maintenance "$maintenance_binary" apply

    database_variables="$(railway_read variable list "${railway_scope[@]}" --service "$database_service" --json)"
    SCOPE_STAGING_DATABASE_PUBLIC_URL="$(
      jq -er '.DATABASE_PUBLIC_URL | strings | select(length > 0)' <<< "$database_variables"
    )"
    export SCOPE_STAGING_DATABASE_PUBLIC_URL
    # Reset the catalog while every metadata writer is still fenced.
    bash .github/scripts/staging-smoke-seed.sh seed
    snapshot_backfill_dir="$(mktemp -d "${RUNNER_TEMP:-/tmp}/scope-repository-snapshot-backfill.XXXXXX")"
    for backfill_command in backfill-landing-files backfill-workflow-catalogs; do
      SCOPE_STAGING_SNAPSHOT_BACKFILL_DIR="$snapshot_backfill_dir" \
        railway run "${railway_scope[@]}" --service "$api_service" --no-local -- \
          sh -c 'DATABASE_URL="$SCOPE_STAGING_DATABASE_PUBLIC_URL" SCOPE_DATA_DIR="$SCOPE_STAGING_SNAPSHOT_BACKFILL_DIR" exec "$@"' \
          scope-maintenance "$maintenance_binary" "$backfill_command"
    done
    rm -rf -- "$snapshot_backfill_dir"
    unset SCOPE_STAGING_DATABASE_PUBLIC_URL

    SCOPE_DEPLOYMENT_COMPONENT=cache bash .github/scripts/deploy-railway.sh "$cache_service" "$1"
    SCOPE_DEPLOYMENT_COMPONENT=worker bash .github/scripts/deploy-railway.sh "$worker_service" "$2"
    SCOPE_DEPLOYMENT_COMPONENT=media bash .github/scripts/deploy-railway.sh "$media_service" "$5"
    SCOPE_DEPLOYMENT_COMPONENT=mediaWorker \
      SCOPE_DEPLOYMENT_EVIDENCE_PATH=.staging-media-worker.ndjson \
      node .github/scripts/deploy-railway-image.mjs "$media_worker_service" "$media_worker_image"
    SCOPE_DEPLOYMENT_COMPONENT=api bash .github/scripts/deploy-railway.sh "$api_service" "$3"
    SCOPE_DEPLOYMENT_COMPONENT=router bash .github/scripts/deploy-railway.sh "$router_service" "$4"
    ;;
  finish)
    if [[ "$#" -ne 1 ]]; then
      echo "usage: deploy-staging-railway.sh finish <web-root>" >&2
      exit 2
    fi
    assert_staging_topology
    SCOPE_DEPLOYMENT_COMPONENT=web bash .github/scripts/deploy-railway.sh "$web_service" "$1"
    evidence_lines="$(mktemp)"
    trap 'rm -f "$evidence_lines"' EXIT
    for service in "$cache_service" "$worker_service" "$media_service" "$media_worker_service" "$api_service" "$router_service" "$web_service"; do
      record_deployment "$service" >> "$evidence_lines"
    done
    jq -s \
      --arg commit "$SCOPE_DEPLOYMENT_SOURCE_SHA" \
      --arg environmentId "$staging_environment_id" \
      --arg workerImage "$media_worker_image" \
      '{commit: $commit, environmentId: $environmentId, workerImage: $workerImage, deployments: .}' \
      "$evidence_lines" > "$evidence_path"
    ;;
  *)
    echo "usage: deploy-staging-railway.sh <prepare|finish> <upload-roots...>" >&2
    exit 2
    ;;
esac
