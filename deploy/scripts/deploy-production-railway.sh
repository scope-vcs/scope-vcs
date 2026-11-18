#!/usr/bin/env bash
set -euo pipefail

manifest_path="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
prepared_release="${SCOPE_PREPARED_RELEASE_PATH:?SCOPE_PREPARED_RELEASE_PATH is required}"
source_sha="${SCOPE_DEPLOYMENT_SOURCE_SHA:?SCOPE_DEPLOYMENT_SOURCE_SHA is required}"
: "${SCOPE_DEPLOYMENT_EVIDENCE_PATH:?SCOPE_DEPLOYMENT_EVIDENCE_PATH is required}"
selected_components="${SCOPE_RELEASE_COMPONENTS:?SCOPE_RELEASE_COMPONENTS is required}"
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || { echo 'A full source revision is required.' >&2; exit 2; }

if [[ -z "${RAILWAY_TOKEN:-}" || -n "${RAILWAY_API_TOKEN:-}" ]]; then
  echo "Production deployment requires only the production RAILWAY_TOKEN." >&2
  exit 1
fi
if [[ -n "${SCOPE_RAILWAY_PREVIEW_ENVIRONMENT_ID:-}" || -n "${SCOPE_ROLE_GRANTS_SQL:-}" ]]; then
  echo "Production deployment refuses preview targets and prepared role grants." >&2
  exit 1
fi

project_id="$(jq -er '.railway.projectId' "$manifest_path")"
production_environment_id="$(jq -er '.environments.production.environmentId' "$manifest_path")"

components_in_dependency_order="$(jq -r --slurpfile release "$prepared_release" --argjson selected "$selected_components" '
  .services | keys_unsorted[] | . as $component
  | select($release[0].components[$component] != null and ($selected | index($component)) != null)
' "$manifest_path")"
[[ -n "$components_in_dependency_order" ]] || { echo 'The prepared release holds none of the selected services.' >&2; exit 2; }
mapfile -t components <<< "$components_in_dependency_order"
node deploy/scripts/railway-artifact.mjs validate "$prepared_release" "$source_sha" "${components[@]}" >/dev/null

if jq -e 'has("maintenanceSha256")' "$prepared_release" >/dev/null; then
  : "${SCOPE_MAINTENANCE_BINARY:?A backend release requires SCOPE_MAINTENANCE_BINARY}"
  run_maintenance() {
    bash deploy/scripts/railway-private-maintenance.sh "$production_environment_id" "$1"
  }
  run_maintenance plan >/dev/null
  run_maintenance apply
  run_maintenance verify
fi

export RAILWAY_PROJECT_ID="$project_id"
export SCOPE_RAILWAY_ENVIRONMENT_ID="$production_environment_id"
source deploy/scripts/railway-service-health.sh
handover="$(mktemp -d)"
trap 'rm -rf -- "$handover"' EXIT

for component in "${components[@]}"; do
  service="$(jq -er --arg component "$component" '.services[$component].id' "$manifest_path")"
  artifact_kind="$(jq -er --arg component "$component" '.services[$component].deployment.artifact.kind' "$manifest_path")"
  export SCOPE_DEPLOYMENT_COMPONENT="$component"
  if [[ "$artifact_kind" == external-image ]]; then
    node deploy/scripts/deploy-railway-image.mjs configure-registry "$service"
    unverified_evidence="$handover/$component.ndjson"
    predecessors="$handover/$component"
    mkdir "$predecessors"
    SCOPE_DEPLOYMENT_EVIDENCE_PATH="$unverified_evidence" SCOPE_PREDECESSOR_TEARDOWN_DIR="$predecessors" \
      node deploy/scripts/deploy-railway-image.mjs "$service" \
      "$(jq -er --arg component "$component" '.components[$component].image' "$prepared_release")"
    deployment_id="$(jq -er '.evidenceId' "$unverified_evidence")"
    expected_config="$(railway_config_path "$component")"
    wait_for_service_health "$service" "$deployment_id" "$expected_config"
    node deploy/scripts/railway-predecessor-teardown.mjs wait "$predecessors"
    wait_for_service_health "$service" "$deployment_id" "$expected_config"
    cat -- "$unverified_evidence" >> "$SCOPE_DEPLOYMENT_EVIDENCE_PATH"
  else
    bash deploy/scripts/deploy-railway.sh "$service"
  fi
done
