#!/usr/bin/env bash
# Public serving configuration is captured in the cutover journal before any replacement.
gates_directory=""

maintenance_gate() {
  env -u RAILWAY_TOKEN RAILWAY_API_TOKEN="$railway_api_token" \
    node .github/scripts/railway-maintenance-gate.mjs "$@"
}

prepare_maintenance_gates() {
  gates_directory="$(mktemp -d "${RUNNER_TEMP:-/tmp}/scope-maintenance-gates.XXXXXX")"
  local component service image
  image="$(jq -er '.components.api.image' "$prepared_release_path")"
  for component in api media-api git-router web; do
    service="$(jq -er --arg component "$component" '.services[$component].id' "$policy_manifest")"
    jq -n --arg projectId "$RAILWAY_PROJECT_ID" --arg environmentId "$environment" \
      --arg serviceId "$service" --arg image "$image" \
      '{projectId:$projectId,environmentId:$environmentId,serviceId:$serviceId,image:$image}' \
      > "$gates_directory/$component.json"
    maintenance_gate snapshot "$gates_directory/$component.json"
  done
  jq -n --slurpfile api "$gates_directory/api.json" \
    --slurpfile media "$gates_directory/media-api.json" \
    --slurpfile router "$gates_directory/git-router.json" --slurpfile web "$gates_directory/web.json" \
    '{api:$api[0],"media-api":$media[0],"git-router":$router[0],web:$web[0]}' > "$gates_directory/snapshots.json"
}

recover_maintenance_gates() {
  local record="$1" component
  gates_directory="$(mktemp -d "${RUNNER_TEMP:-/tmp}/scope-maintenance-gates.XXXXXX")"
  for component in api media-api git-router web; do
    jq -e --arg component "$component" '.maintenanceGates[$component] | select(.previous != null)' \
      <<< "$record" > "$gates_directory/$component.json"
  done
}

enter_public_gates() {
  [[ -n "$gates_directory" ]] || return 0
  local component failed=0
  mark_maintenance_start
  # Gate activation can remove old deployment IDs. Recovery must now finish the pinned
  # release forward, even if its schema transaction has not started yet.
  cutover_committed=1
  for component in web git-router media-api api; do
    if ! maintenance_gate reclose "$gates_directory/$component.json"; then
      failed=1
      continue
    fi
    case "$component" in
      api) api_closed=1 ;;
      media-api) media_closed=1 ;;
    esac
  done
  return "$failed"
}

restore_candidate_configuration() {
  local component="$1"
  [[ -n "$gates_directory" && -f "$gates_directory/$component.json" ]] || return 0
  maintenance_gate restore "$gates_directory/$component.json"
}
