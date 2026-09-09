#!/usr/bin/env bash
set -euo pipefail

manifest_path="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
# shellcheck source=.github/scripts/railway-graphql.sh
source "$(dirname "${BASH_SOURCE[0]}")/railway-graphql.sh"

if [[ -z "${RAILWAY_API_TOKEN:-}" || -n "${RAILWAY_TOKEN:-}" ]]; then
  echo "Stopping staging writers requires only RAILWAY_API_TOKEN." >&2
  exit 1
fi

project_id="$(jq -er '.railway.projectId' "$manifest_path")"
production_environment_id="$(jq -er '.environments.production.environmentId' "$manifest_path")"
staging_environment_id="$(jq -er '.environments.staging.environmentId' "$manifest_path")"
worker_service="$(jq -er '.services["run-worker"].id' "$manifest_path")"
api_service="$(jq -er '.services.api.id' "$manifest_path")"
cache_service="$(jq -er '.services.cache.id' "$manifest_path")"
media_service="$(jq -er '.services["media-api"].id | strings | select(length > 0)' "$manifest_path")"
media_worker_service="$(jq -er '.services["media-worker"].id | strings | select(length > 0)' "$manifest_path")"

if [[ "$staging_environment_id" == "$production_environment_id" ]]; then
  echo "Staging environment matches production." >&2
  exit 1
fi

railway_scope=(
  --project "$project_id"
  --environment "$staging_environment_id"
)

remove_deployment() {
  local deployment_id="$1"
  local response
  # shellcheck disable=SC2016
  response="$(railway_graphql once \
    'mutation DeploymentRemove($id: String!) { deploymentRemove(id: $id) }' \
    "$(jq -cn --arg id "$deployment_id" '{id: $id}')")" || return $?
  jq -e '.data.deploymentRemove == true' <<< "$response" >/dev/null
}

stop_service() {
  local service="$1"
  local deployments deployment_id
  deployments="$(
    node .github/scripts/railway-read.mjs deployment list "${railway_scope[@]}" --service "$service" --limit 10 --json
  )" || return $?
  deployment_id="$(
    jq -er '
      first(
        .[] | select(
          .status != "REMOVED" and .status != "FAILED" and .status != "CRASHED" and
          .status != "SKIPPED"
        )
      ).id // ""
    ' <<< "$deployments"
  )" || return $?
  if [[ -n "$deployment_id" ]]; then
    local _attempt remaining
    for _attempt in 1 2 3; do
      if remove_deployment "$deployment_id"; then return 0; fi
      deployments="$(node .github/scripts/railway-read.mjs deployment list "${railway_scope[@]}" --service "$service" --limit 10 --json)" || return $?
      remaining="$(jq -er --arg id "$deployment_id" 'map(select(.id == $id and .status != "REMOVED")) | length' <<< "$deployments")" || return $?
      [[ "$remaining" == 0 ]] && return 0
      sleep 2
    done
    echo "Could not confirm staging deployment removal." >&2
    return 1
  fi
}

wait_until_stopped() {
  local service="$1"
  local deadline=$((SECONDS + 300))
  local running crashed

  while [[ "$SECONDS" -lt "$deadline" ]]; do
    local services replicas
    services="$(node .github/scripts/railway-read.mjs service list "${railway_scope[@]}" --json)" || return $?
    replicas="$(jq -er --arg service "$service" '
      .[] | select(.id == $service) |
      [(.replicas.running // 0), (.replicas.crashed // 0)] | @tsv
    ' <<< "$services")" || return $?
    IFS=$'\t' read -r running crashed <<< "$replicas"
    if [[ "$running" == "0" && "$crashed" == "0" ]]; then
      return 0
    fi
    sleep 5
  done

  echo "Timed out waiting for staging service $service to stop." >&2
  return 1
}

stop_service "$api_service"
stop_service "$worker_service"
stop_service "$cache_service"
stop_service "$media_service"
stop_service "$media_worker_service"
wait_until_stopped "$api_service"
wait_until_stopped "$worker_service"
wait_until_stopped "$cache_service"
wait_until_stopped "$media_service"
wait_until_stopped "$media_worker_service"
