#!/usr/bin/env bash
set -euo pipefail

service_name="${1:?usage: deploy-railway.sh <service-id>}"

if [[ -z "${RAILWAY_API_TOKEN:-}" && -z "${RAILWAY_TOKEN:-}" ]]; then
  echo "Set RAILWAY_API_TOKEN or RAILWAY_TOKEN before deploying ${service_name}."
  exit 1
fi

if [[ -n "${RAILWAY_API_TOKEN:-}" && -n "${RAILWAY_TOKEN:-}" ]]; then
  echo "Set only one Railway token type before deploying ${service_name}."
  exit 1
fi

if [ -z "${RAILWAY_PROJECT_ID:-}" ]; then
  echo "Set RAILWAY_PROJECT_ID before deploying ${service_name}."
  exit 1
fi

railway_environment="${SCOPE_RAILWAY_ENVIRONMENT_ID:?SCOPE_RAILWAY_ENVIRONMENT_ID is required}"
prepared_release="${SCOPE_PREPARED_RELEASE_PATH:?SCOPE_PREPARED_RELEASE_PATH is required}"
deployment_component="${SCOPE_DEPLOYMENT_COMPONENT:-}"
deployment_source_sha="${SCOPE_DEPLOYMENT_SOURCE_SHA:-${GITHUB_SHA:-}}"
deployment_evidence_path="${SCOPE_DEPLOYMENT_EVIDENCE_PATH:-}"
defer_service_health="${SCOPE_DEFER_SERVICE_HEALTH:-0}"
predecessor_teardown_dir="${SCOPE_PREDECESSOR_TEARDOWN_DIR:-}"
railway_scope=(--project "$RAILWAY_PROJECT_ID" --environment "$railway_environment")
expected_config=""

if [[ "$defer_service_health" != "0" && "$defer_service_health" != "1" ]]; then
  echo "SCOPE_DEFER_SERVICE_HEALTH must be 0 or 1." >&2
  exit 2
fi

source "$(dirname "${BASH_SOURCE[0]}")/railway-service-health.sh"

ensure_service_exists() {
  local services_json
  services_json="$(railway_read service list "${railway_scope[@]}" --json)"
  if ! jq -e --arg service "$service_name" 'any(.[]; .name == $service or .id == $service)' \
    <<< "$services_json" >/dev/null; then
    echo "Railway service '${service_name}' was not found in environment '${railway_environment}'."
    echo "Create the service in Railway, configure its variables, then rerun this workflow."
    return 1
  fi
}

print_deployment_logs() {
  local kind
  for kind in build deployment; do
    echo "::group::Railway ${kind/deployment/deploy} logs for ${service_name}/$1"
    railway logs "$1" --project "$RAILWAY_PROJECT_ID" --service "$service_name" \
      --environment "$railway_environment" "--$kind" --lines 200 || true
    echo "::endgroup::"
  done
}

record_deployment_evidence() {
  [[ -n "$deployment_evidence_path" ]] || return 0
  node .github/scripts/railway-artifact.mjs validate "$prepared_release" "$deployment_source_sha" "$deployment_component" >/dev/null
  jq -cn --arg component "$deployment_component" --arg sourceSha "$deployment_source_sha" --arg evidenceId "$1" \
    '{component: $component, sourceSha: $sourceSha, provider: "railway", evidenceId: $evidenceId}' \
    >> "$deployment_evidence_path"
}

wait_for_deployment() {
  local deadline=$((SECONDS + 900)) deployment_json deployment_status
  while true; do
    if deployment_json="$(railway_read deployment list --project "$RAILWAY_PROJECT_ID" --service "$service_name" \
      --environment "$railway_environment" --limit 10 --json)"; then
      deployment_status="$(jq -r --arg id "$deployment_id" 'first(.[] | select(.id == $id)) | .status' <<< "$deployment_json")"
      if [[ -n "$deployment_status" ]]; then
        echo "Railway deployment $deployment_id is $deployment_status."
        case "$deployment_status" in
          SUCCESS)
            return 0
            ;;
          SKIPPED)
            echo "Prepared activation $deployment_id was skipped; refusing to substitute another deployment." >&2
            return 1
            ;;
          FAILED|CRASHED|REMOVED)
            print_deployment_logs "$deployment_id"
            return 1
            ;;
        esac
      else
        echo "Waiting for Railway deployment $deployment_id to appear..."
      fi
    else
      echo "Waiting for Railway deployment status..."
    fi

    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for Railway deployment."
      return 1
    fi

    sleep 10
  done
}

ensure_service_exists

[[ -n "$deployment_component" && -n "$deployment_source_sha" ]] || {
  echo 'Prepared activation requires a component and exact source revision.' >&2
  exit 2
}
node .github/scripts/railway-artifact.mjs validate "$prepared_release" "$deployment_source_sha" "$deployment_component" >/dev/null
expected_service="$(jq -er --arg component "$deployment_component" '.components[$component].serviceId' "$prepared_release")"
[[ "$expected_service" == "$service_name" ]] || {
  echo 'Prepared artifact service does not match the activation target.' >&2
  exit 2
}
if [[ "$(node .github/scripts/deployment-components.mjs describe "$deployment_component" | jq -r '.verifyTransitionConfig')" == true ]]; then
  expected_config="$(railway_config_path "$deployment_component")"
fi
owned_teardown_dir=""
if [[ -z "$predecessor_teardown_dir" ]]; then
  owned_teardown_dir="$(mktemp -d)"
  predecessor_teardown_dir="$owned_teardown_dir"
fi
deployed_metadata="$(mktemp)"
trap 'rm -rf -- "$deployed_metadata" ${owned_teardown_dir:+"$owned_teardown_dir"}' EXIT
node .github/scripts/railway-predecessor-teardown.mjs snapshot \
  "$predecessor_teardown_dir" "$deployment_component" "$service_name"
deploy_output="$(node .github/scripts/railway-artifact.mjs activate \
  "$prepared_release" "$deployment_component" "$railway_environment")"
printf '%s\n' "$deploy_output"
deployment_id="$(printf '%s\n' "$deploy_output" | jq -er 'select(.deploymentId | type == "string" and length > 0) | .deploymentId' | tail -1)"

wait_for_deployment
railway_read deployment list "${railway_scope[@]}" --service "$service_name" --limit 100 --json > "$deployed_metadata"
node .github/scripts/railway-artifact.mjs verify "$prepared_release" \
  "$deployment_component" "$deployed_metadata" "$deployment_id"
node .github/scripts/railway-predecessor-teardown.mjs activated \
  "$predecessor_teardown_dir" "$deployment_component" "$deployment_id"
if [[ "$defer_service_health" == "0" ]]; then
  wait_for_service_health "$service_name" "$deployment_id" "$expected_config"
fi
if [[ -n "${SCOPE_RELEASE_DEPLOYMENTS_FILE:-}" ]]; then
  (
    flock -x 9
    jq --arg component "$deployment_component" --arg id "$deployment_id" \
      '.[$component] = $id' "$SCOPE_RELEASE_DEPLOYMENTS_FILE" > "$SCOPE_RELEASE_DEPLOYMENTS_FILE.tmp.$$"
    mv "$SCOPE_RELEASE_DEPLOYMENTS_FILE.tmp.$$" "$SCOPE_RELEASE_DEPLOYMENTS_FILE"
  ) 9>>"$SCOPE_RELEASE_DEPLOYMENTS_FILE.lock"
fi
record_deployment_evidence "$deployment_id"

[[ -z "$owned_teardown_dir" ]] || node .github/scripts/railway-predecessor-teardown.mjs wait "$owned_teardown_dir"
