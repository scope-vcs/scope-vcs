#!/usr/bin/env bash
set -euo pipefail

manifest_path="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
maintenance_binary="${SCOPE_MAINTENANCE_BINARY:-./target/release/scope-maintenance}"
evidence_path="${SCOPE_STAGING_EVIDENCE_PATH:-staging-deployments.json}"
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
staging_router_url="https://$(jq -er '.environments.staging.routerDomain' "$manifest_path")"
worker_service="$(jq -er '.services["run-worker"].id' "$manifest_path")"
api_service="$(jq -er '.services.api.id' "$manifest_path")"
router_service="$(jq -er '.services["git-router"].id' "$manifest_path")"
media_service="$(jq -er '.services["media-api"].id' "$manifest_path")"
media_worker_service="$(jq -er '.services["media-worker"].id' "$manifest_path")"
if [[ "$staging_environment_id" == "$production_environment_id" ]]; then
  echo "Staging environment matches production." >&2
  exit 1
fi

source "$(dirname "${BASH_SOURCE[0]}")/railway-service-health.sh"

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

assert_writers_stopped() {
  railway_read service list "${railway_scope[@]}" --json | jq -e \
    --arg api "$api_service" --arg worker "$worker_service" \
    --arg media "$media_service" --arg media_worker "$media_worker_service" '
      . as $services | all($api, $worker, $media, $media_worker;
        . as $id | first($services[] | select(.id == $id)) // null
        | . != null and (.replicas.running // 0) == 0 and (.replicas.crashed // 0) == 0)
    ' >/dev/null
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

: "${SCOPE_PREPARED_RELEASE_PATH:?Prepared release manifest is required}"
node .github/scripts/railway-artifact.mjs validate "$SCOPE_PREPARED_RELEASE_PATH" "$SCOPE_DEPLOYMENT_SOURCE_SHA" >/dev/null

if jq -e '.components.api' "$SCOPE_PREPARED_RELEASE_PATH" >/dev/null; then
  node .github/scripts/railway-artifact.mjs verify-maintenance "$SCOPE_PREPARED_RELEASE_PATH" "$maintenance_binary" >/dev/null
  assert_writers_stopped
  run_maintenance() {
    SCOPE_MAINTENANCE_BINARY="$maintenance_binary" \
      bash .github/scripts/railway-private-maintenance.sh "$staging_environment_id" "$1"
  }
  if [[ "${SCOPE_STAGING_RESUME:-0}" == 1 ]]; then
    run_maintenance plan | jq -e '.exact == true and .pending == []' >/dev/null
  else
    run_maintenance plan >/dev/null
    run_maintenance validate-workflow-catalogs
    run_maintenance apply
  fi
  run_maintenance verify
fi

evidence_dir="$(mktemp -d)"
predecessor_teardown_dir="$(mktemp -d)"
trap 'rm -rf "$evidence_dir" "$predecessor_teardown_dir"' EXIT
export SCOPE_PREDECESSOR_TEARDOWN_DIR="$predecessor_teardown_dir"
activate_staging() {
  local component="$1" service image
  jq -e --arg component "$component" '.components[$component]' "$SCOPE_PREPARED_RELEASE_PATH" >/dev/null || return 0
  service="$(jq -er --arg component "$component" '.services[$component].id' "$manifest_path")"
  export SCOPE_DEPLOYMENT_COMPONENT="$component"
  export SCOPE_DEPLOYMENT_EVIDENCE_PATH="$evidence_dir/$component.jsonl"
  case "$component" in
    media-worker)
      image="$(jq -er '.components["media-worker"].image' "$SCOPE_PREPARED_RELEASE_PATH")"
      node .github/scripts/deploy-railway-image.mjs "$service" "$image"
      ;;
    *)
      bash .github/scripts/deploy-railway.sh "$service"
      ;;
  esac
  jq -se --arg component "$component" --arg sha "$SCOPE_DEPLOYMENT_SOURCE_SHA" '
    length == 1 and .[0].component == $component and .[0].sourceSha == $sha
    and (.[0].evidenceId | type == "string" and length > 0)
  ' "$SCOPE_DEPLOYMENT_EVIDENCE_PATH" >/dev/null
}

activate_staging_pair() {
  local first_pid second_pid failed=0
  (activate_staging "$1") &
  first_pid=$!
  (activate_staging "$2") &
  second_pid=$!
  wait "$first_pid" || failed=1
  wait "$second_pid" || failed=1
  return "$failed"
}

activate_staging media-api
activate_staging_pair run-worker media-worker
activate_staging api
activate_staging git-router
activate_staging web
assert_staging_topology
node .github/scripts/railway-predecessor-teardown.mjs wait "$predecessor_teardown_dir"
evidence_files=()
for component in run-worker media-api media-worker api git-router web; do
  [[ ! -f "$evidence_dir/$component.jsonl" ]] || evidence_files+=("$evidence_dir/$component.jsonl")
done
jq -s --slurpfile manifest "$manifest_path" --arg commit "$SCOPE_DEPLOYMENT_SOURCE_SHA" --arg environmentId "$staging_environment_id" \
  '{commit: $commit, environmentId: $environmentId, candidateDeployments: 1,
    deployments: map({service: $manifest[0].services[.component].id, deploymentId: .evidenceId, status: "SUCCESS"})}' \
  "${evidence_files[@]}" > "$evidence_path"
