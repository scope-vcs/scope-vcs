#!/usr/bin/env bash
# Shared Railway service health polling for deployment scripts. Source this file.
# Requires RAILWAY_PROJECT_ID and SCOPE_RAILWAY_ENVIRONMENT_ID in the environment.

railway_service_health_scripts="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Prints the checked-in Railway config path for a manifest component.
railway_config_path() {
  local component="$1" manifest directory
  manifest="${SCOPE_DEPLOYMENT_MANIFEST:-$railway_service_health_scripts/../deployment-services.json}"
  directory="$(jq -er --arg component "$component" '.services[$component].sourceDirectory' "$manifest")" || {
    echo "Deployment manifest has no source directory for $component." >&2
    return 1
  }
  printf '%s\n' "$directory/railway.json"
}

# service_is_healthy SERVICE [EXPECTED_DEPLOYMENT_ID] [EXPECTED_CONFIG_PATH]
service_is_healthy() {
  local service="$1" expected_deployment_id="${2:-}" expected_config="${3:-}" services_json
  services_json="$(
    node "$railway_service_health_scripts/railway-read.mjs" status \
      --project "$RAILWAY_PROJECT_ID" \
      --environment "$SCOPE_RAILWAY_ENVIRONMENT_ID" \
      --json
  )"
  SCOPE_RAILWAY_ENVIRONMENT_ID="$SCOPE_RAILWAY_ENVIRONMENT_ID" \
    SCOPE_EXPECTED_RAILWAY_CONFIG="$expected_config" \
    SCOPE_RAILWAY_SERVICES_JSON="$services_json" \
    SCOPE_RAILWAY_SERVICE_ID="$service" \
    SCOPE_EXPECTED_RAILWAY_DEPLOYMENT_ID="$expected_deployment_id" \
    node "$railway_service_health_scripts/railway-service-health.mjs" >/dev/null
}

# wait_for_service_health SERVICE [EXPECTED_DEPLOYMENT_ID] [EXPECTED_CONFIG_PATH]
wait_for_service_health() {
  local service="$1" expected_deployment_id="${2:-}" expected_config="${3:-}"
  local timeout="${SCOPE_SERVICE_HEALTH_TIMEOUT_SECONDS:-600}"
  local interval="${SCOPE_SERVICE_HEALTH_POLL_SECONDS:-10}"
  local deadline=$((SECONDS + timeout))
  while true; do
    if service_is_healthy "$service" "$expected_deployment_id" "$expected_config" 2>/dev/null; then
      return 0
    fi
    (( SECONDS < deadline )) || break
    sleep "$interval"
  done
  service_is_healthy "$service" "$expected_deployment_id" "$expected_config" || true
  echo "Timed out waiting for $service to reach its exact healthy deployment." >&2
  return 1
}
