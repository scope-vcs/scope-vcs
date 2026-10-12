#!/usr/bin/env bash
set -euo pipefail

maintenance_binary="${SCOPE_MAINTENANCE_BINARY:-./target/release/scope-maintenance}"
environment="${SCOPE_RAILWAY_ENVIRONMENT_ID:?SCOPE_RAILWAY_ENVIRONMENT_ID is required}"
api_service="${SCOPE_RAILWAY_API_SERVICE_ID:?SCOPE_RAILWAY_API_SERVICE_ID is required}"
worker_service="${SCOPE_RAILWAY_WORKER_SERVICE_ID:?SCOPE_RAILWAY_WORKER_SERVICE_ID is required}"
router_service="${SCOPE_RAILWAY_ROUTER_SERVICE_ID:?SCOPE_RAILWAY_ROUTER_SERVICE_ID is required}"
media_service="${SCOPE_RAILWAY_MEDIA_SERVICE_ID:?SCOPE_RAILWAY_MEDIA_SERVICE_ID is required}"
media_worker_service="${SCOPE_RAILWAY_MEDIA_WORKER_SERVICE_ID:?SCOPE_RAILWAY_MEDIA_WORKER_SERVICE_ID is required}"
router_group="${SCOPE_RAILWAY_ROUTER_GROUP_ID:?SCOPE_RAILWAY_ROUTER_GROUP_ID is required}"
: "${SCOPE_RAILWAY_DATABASE_SERVICE_ID:?SCOPE_RAILWAY_DATABASE_SERVICE_ID is required}"
api_region="${SCOPE_RAILWAY_API_REGION_ID:?SCOPE_RAILWAY_API_REGION_ID is required}"
: "${SCOPE_RAILWAY_WORKER_REGION_ID:?SCOPE_RAILWAY_WORKER_REGION_ID is required}"
: "${SCOPE_RAILWAY_MEDIA_REGION_ID:?SCOPE_RAILWAY_MEDIA_REGION_ID is required}"
media_worker_image="${SCOPE_MEDIA_WORKER_IMAGE:-}"
deploy_worker_requested="${SCOPE_DEPLOY_WORKER:-1}"
deploy_router_requested="${SCOPE_DEPLOY_ROUTER:-1}"
deploy_api_requested="${SCOPE_DEPLOY_API:-1}"
deploy_media_requested="${SCOPE_DEPLOY_MEDIA:-1}"
deploy_media_worker_requested="${SCOPE_DEPLOY_MEDIA_WORKER:-1}"
successful_deployments="${SCOPE_SUCCESSFUL_DEPLOYMENTS:-}"
[[ -n "$successful_deployments" ]] || successful_deployments='{}'
deployment_evidence_path="${SCOPE_DEPLOYMENT_EVIDENCE_PATH:-}"
pending_evidence_path=""
predecessor_teardown_dir=""
remove_pending_evidence() {
  [[ -z "$pending_evidence_path" ]] ||
    rm -f -- "$pending_evidence_path".{run-worker,media-api,media-worker,api,git-router,web}
}
if [[ -n "$deployment_evidence_path" ]]; then
  pending_evidence_path="${deployment_evidence_path}.pending.$$"
  remove_pending_evidence
fi

for deployment_flag in deploy_worker_requested deploy_router_requested \
  deploy_api_requested deploy_media_requested deploy_media_worker_requested; do
  if [[ "${!deployment_flag}" != "0" && "${!deployment_flag}" != "1" ]]; then
    echo "${deployment_flag} must be 0 or 1." >&2
    exit 2
  fi
done
if [[ "$deploy_worker_requested$deploy_router_requested$deploy_api_requested$deploy_media_requested$deploy_media_worker_requested" == 00000 ]]; then
  echo "At least one backend service must be selected for deployment." >&2
  exit 2
fi
if [[ ! "$media_worker_image" =~ ^ghcr\.io/scope-vcs/scope-media-worker@sha256:[0-9a-f]{64}$ ]]; then
  echo "SCOPE_MEDIA_WORKER_IMAGE must pin the reviewed GHCR image by sha256 digest." >&2
  exit 2
fi

if [[ -z "${RAILWAY_TOKEN:-}" || -z "${RAILWAY_API_TOKEN:-}" || -z "${RAILWAY_PROJECT_ID:-}" ]]; then
  echo "RAILWAY_TOKEN, RAILWAY_API_TOKEN, and RAILWAY_PROJECT_ID are required for backend deployment." >&2
  exit 1
fi
if [[ ! -x "$maintenance_binary" ]]; then
  echo "Maintenance binary is not executable: $maintenance_binary" >&2
  exit 1
fi

railway_api_token="$RAILWAY_API_TOKEN"
unset RAILWAY_API_TOKEN

policy_manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
web_service="$(jq -er .services.web.id "$policy_manifest")"

railway_scope=(--project "$RAILWAY_PROJECT_ID" --environment "$environment")
cutover_committed=0
api_closed=0
worker_closed=0
media_closed=0
media_worker_closed=0
media_had_history=1
media_worker_had_history=1

maintenance() {
  SCOPE_MAINTENANCE_BINARY="$maintenance_binary" \
    bash .github/scripts/railway-private-maintenance.sh "$environment" "$1"
}

maintenance_read() {
  local command="$1"
  local attempt output
  for attempt in 1 2 3; do
    if output="$(maintenance "$command")"; then
      printf '%s\n' "$output"
      return 0
    fi
    if [[ "$attempt" -lt 3 ]]; then
      echo "Maintenance $command failed; retrying read-only database access." >&2
      sleep 2
    fi
  done
  return 1
}

wait_for_writer_fence() {
  local grace_seconds="${SCOPE_WRITER_FENCE_GRACE_SECONDS:-10}"
  local grace_deadline=$((SECONDS + grace_seconds))
  local timeout_seconds
  timeout_seconds="$(jq -er '.releasePolicy.writerDrainTimeoutSeconds | select(type == "number" and . > 0 and floor == .)' "$policy_manifest")"
  local deadline=$((SECONDS + timeout_seconds))
  local drained=0
  while (( SECONDS < deadline )); do
    if maintenance fence; then
      return 0
    fi
    if [[ "$drained" == "0" && "$SECONDS" -ge "$grace_deadline" ]]; then
      maintenance drain-writers
      drained=1
    fi
    echo "Metadata writers are still draining; retrying the fence probe." >&2
    sleep 2
  done
  echo "Timed out waiting for metadata writers to release the database fence." >&2
  return 1
}

plan_requires_maintenance() {
  jq -e '.pending | if type == "array" then length > 0 else "" | halt_error(2) end' <<< "$1" >/dev/null
}

plan_is_exact() {
  jq -e '.exact == true and .pending == []' <<< "$1" >/dev/null
}

plans_have_same_ledger() {
  jq -en --argjson before "$1" --argjson after "$2" '
    def ledger: if (.pending | type) == "array" then [.pending[].name] else null end;
    $before.exact == $after.exact
      and ($before.applied | type) == "array" and $before.applied == $after.applied
      and ($before | ledger) != null and ($before | ledger) == ($after | ledger)
  ' >/dev/null 2>&1
}

successful_deployment_field() {
  jq -er --arg component "$1" --arg field "$2" '
    .[$component] as $deployment
    | if $deployment == null then ""
      elif $deployment.provider == "railway"
        and ($deployment[$field] | type) == "string"
        and ($deployment[$field] | length) > 0
      then $deployment[$field]
      else error("invalid durable Railway deployment for \($component)")
      end
  ' <<< "$successful_deployments"
}

require_successful_deployment_id() {
  local component="$1"
  local deployment_id
  deployment_id="$(successful_deployment_field "$component" evidenceId)"
  if [[ -z "$deployment_id" ]]; then
    echo "No durable Railway deployment identity exists for $component." >&2
    return 1
  fi
  printf '%s\n' "$deployment_id"
}

carried_service_is_healthy() {
  local component="$1"
  local service_name="$2"
  local expected_deployment_id
  expected_deployment_id="$(require_successful_deployment_id "$component")" || return 1
  service_is_healthy "$service_name" "$expected_deployment_id"
}

close_writer() {
  local service="$1" flag="$2" public="$3" id state
  [[ "${!flag}" == "0" ]] || return 0
  id="$(deployment_id "$service")" || return 1
  if [[ "$public" == 1 && -n "$gates_directory" ]]; then
    state="$(railway deployment list "${railway_scope[@]}" --service "$service" --limit 100 --json |
      jq -er --arg id "$id" --arg image "$(jq -er .components.api.image "$prepared_release_path")" '
        [.[] | select(.id == $id)] | if length != 1 then error("missing current deployment") else .[0] end
        | .meta as $metadata | $metadata.serviceManifest as $manifest
        | if ($manifest.deploy.startCommand | type) != "string" then error("unknown serving configuration")
          elif $manifest.deploy.startCommand == "/app/bin/scope-maintenance serve" then
            if $manifest.source.image == $image or $metadata.imageDigest == ($image | split("@")[1])
            then "gate" else error("unknown maintenance image") end
          else "writer" end
      ')" || return 1
    if [[ "$state" == gate ]]; then
      printf -v "$flag" 1
      return 0
    fi
  fi
  deployment_action Stop "$service" "$id" || return 1
  printf -v "$flag" 1
}

quiesce_writers() {
  local failed=0
  enter_public_gates || failed=1
  close_writer "$api_service" api_closed 1 || failed=1
  close_writer "$worker_service" worker_closed 0 || failed=1
  close_writer "$media_service" media_closed 1 || failed=1
  close_writer "$media_worker_service" media_worker_closed 0 || failed=1
  wait_for_writer_fence || failed=1
  return "$failed"
}

restore_writer() {
  local flag="$1" service="$2" component="$3" had_history="${4:-1}"
  [[ "${!flag}" == "1" ]] || return 0
  [[ "$had_history" != "1" ]] || restart_service "$service" "$(require_successful_deployment_id "$component")"
  printf -v "$flag" 0
}

restore_old_release() {
  echo "Migration did not commit; restoring the previous metadata-writer deployments." >&2
  restore_writer worker_closed "$worker_service" run-worker
  restore_writer api_closed "$api_service" api
  restore_writer media_closed "$media_service" media-api "$media_had_history"
  restore_writer media_worker_closed "$media_worker_service" media-worker "$media_worker_had_history"
  mark_maintenance_end
}

deploy_release() {
  successful_deployment_field "$1" sourceSha >/dev/null
  SCOPE_DEPLOYMENT_COMPONENT="$1" \
    SCOPE_DEPLOYMENT_EVIDENCE_PATH="$(component_pending_evidence_path "$1")" \
    SCOPE_PREDECESSOR_TEARDOWN_DIR="$predecessor_teardown_dir" \
    SCOPE_DEFER_SERVICE_HEALTH=1 \
    bash .github/scripts/deploy-railway.sh "$2"
}

pending_deployment_id() {
  local evidence_file
  evidence_file="$(component_pending_evidence_path "$1")"
  [[ -n "$evidence_file" && -s "$evidence_file" ]] || return 0
  jq -sjer --arg component "$1" '
    if length == 1 and .[0].component == $component and (.[0].evidenceId | type == "string" and length > 0)
    then .[0].evidenceId else error("invalid pending deployment evidence") end
  ' "$evidence_file"
}

component_pending_evidence_path() {
  [[ -z "$pending_evidence_path" ]] || printf '%s.%s\n' "$pending_evidence_path" "$1"
}

activate_release() {
  local component="$1"
  local service_name="$2"
  local actual_deployment_id expected_deployment_id
  restore_candidate_configuration "$component"
  deploy_release "$component" "$service_name"
  expected_deployment_id="$(pending_deployment_id "$component")"
  [[ -n "$expected_deployment_id" ]] \
    || expected_deployment_id="$(require_successful_deployment_id "$component")"
  actual_deployment_id="$(deployment_id "$service_name")"
  if [[ "$actual_deployment_id" != "$expected_deployment_id" ]]; then
    echo "Refusing to activate $service_name: active deployment $actual_deployment_id does not match expected deployment $expected_deployment_id." >&2
    return 1
  fi
  if [[ "$(running_replicas "$service_name")" == "0" ]]; then
    deployment_action Restart "$service_name" "$expected_deployment_id"
  fi
  wait_for_service_health "$service_name" "$expected_deployment_id" "$(railway_config_path "$component")"
}

activate_image_release() {
  local component="$1"
  local service_name="$2"
  local image="$3"
  local expected_deployment_id
  RAILWAY_API_TOKEN="$railway_api_token" \
    SCOPE_DEPLOYMENT_COMPONENT="$component" \
    SCOPE_DEPLOYMENT_EVIDENCE_PATH="$(component_pending_evidence_path "$component")" \
    SCOPE_PREDECESSOR_TEARDOWN_DIR="$predecessor_teardown_dir" \
    node .github/scripts/deploy-railway-image.mjs "$service_name" "$image"
  expected_deployment_id="$(pending_deployment_id "$component")"
  [[ -n "$expected_deployment_id" ]] || {
    echo "Pinned image deployment produced no Railway evidence for $component." >&2
    return 1
  }
  wait_for_service_health "$service_name" "$expected_deployment_id" "$(railway_config_path "$component")"
}

promote_pending_evidence() {
  [[ -n "$deployment_evidence_path" ]] || return 0
  local component paths=()
  for component in run-worker media-api media-worker api git-router web; do
    [[ ! -e "$pending_evidence_path.$component" ]] || paths+=("$pending_evidence_path.$component")
  done
  (( ${#paths[@]} > 0 )) || return 0
  cat -- "${paths[@]}" >> "$deployment_evidence_path"
  rm -f -- "${paths[@]}"
}

discard_pending_evidence() {
  remove_pending_evidence
  [[ -z "$predecessor_teardown_dir" ]] || rm -rf -- "$predecessor_teardown_dir"
}

activate_pair() {
  local first_pid second_pid failed=0
  (trap - EXIT; "$1" "$2" "$3" "$4") &
  first_pid=$!
  (trap - EXIT; "$5" "$6" "$7" "$8") &
  second_pid=$!
  wait "$first_pid" || failed=1
  wait "$second_pid" || failed=1
  return "$failed"
}

deploy_selected_releases() {
  if [[ "$deploy_worker_requested" == "1" ]]; then
    activate_release run-worker "$worker_service"
    promote_pending_evidence
  fi
  if [[ "$deploy_media_requested" == "1" ]]; then
    activate_release media-api "$media_service"
    promote_pending_evidence
  fi
  if [[ "$deploy_media_worker_requested" == "1" ]]; then
    activate_image_release media-worker "$media_worker_service" "$media_worker_image"
    promote_pending_evidence
  fi
  if [[ "$deploy_api_requested" == "1" ]]; then
    activate_release api "$api_service"
    promote_pending_evidence
  fi
}

deploy_and_reopen() {
  cutover_phase verifying
  maintenance_read verify
  predecessor_teardown_dir="$(mktemp -d)"
  cutover_phase activating-media
  media_closed=0
  activate_release media-api "$media_service"
  cutover_phase activating-worker
  worker_closed=0
  cutover_phase activating-media-worker
  media_worker_closed=0
  activate_pair activate_release run-worker "$worker_service" '' \
    activate_image_release media-worker "$media_worker_service" "$media_worker_image"
  cutover_phase activating-api
  api_closed=0
  activate_release api "$api_service"
  maintenance_read verify
  cutover_phase activating-router
  activate_release git-router "$router_service"
  assert_router_topology
  cutover_phase activating-web
  restore_candidate_configuration web
  deploy_release web "$web_service"
  local web_deployment_id
  web_deployment_id="$(pending_deployment_id web)"
  wait_for_service_health "$web_service" "$web_deployment_id" "$(railway_config_path web)"
  node .github/scripts/railway-predecessor-teardown.mjs wait "$predecessor_teardown_dir"
  mark_maintenance_end
  promote_pending_evidence
}

leave_failure_state() {
  local exit_status="$1"
  trap - EXIT
  if [[ "$exit_status" -ne 0 ]]; then
    if [[ "$cutover_committed" == "0" \
      && ( "$api_closed" == "1" || "$worker_closed" == "1" \
      || "$media_closed" == "1" || "$media_worker_closed" == "1" ) ]]; then
      local fresh_plan
      fresh_plan="$(maintenance_read plan || true)"
      if [[ -n "$fresh_plan" ]] && plans_have_same_ledger "$plan_json" "$fresh_plan"; then
        cutover_phase restoring && restore_old_release && cutover_phase restored \
          || echo "Failed to restore the previous release; recovery remains required." >&2
      else
        cutover_committed=1
        quiesce_writers || true
        echo "Ledger is uncertain; writers remain closed." >&2
      fi
    elif [[ "$cutover_committed" == "1" ]]; then
      if quiesce_writers; then
        echo "Cutover requires forward recovery; writers are closed. Rerun this workflow to finish the pinned deployment." >&2
      else
        echo "Failed to re-close metadata writers after the cutover. Writer closure is unverified; investigate the failed shutdown before recovery." >&2
      fi
    fi
  fi
  discard_pending_evidence
  exit "$exit_status"
}
trap 'leave_failure_state $?' EXIT

finish() {
  trap - EXIT
  discard_pending_evidence
  exit 0
}

source .github/scripts/railway-service-health.sh
source .github/scripts/railway-backend-control.sh
source .github/scripts/release-maintenance-gates.sh
source .github/scripts/release-cutover.sh

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  printf '| Release phase | Seconds |\n| --- | ---: |\n' >> "$GITHUB_STEP_SUMMARY"
fi
if [[ -n "$cutover_id" ]]; then
  validate_production_target
  recover_cutover
  finish
fi
journal cutover-guard
selected_components=()
[[ "$deploy_api_requested" == "0" ]] || selected_components+=(api)
[[ "$deploy_worker_requested" == "0" ]] || selected_components+=(run-worker)
[[ "$deploy_router_requested" == "0" ]] || selected_components+=(git-router)
[[ "$deploy_media_requested" == "0" ]] || selected_components+=(media-api)
[[ "$deploy_media_worker_requested" == "0" ]] || selected_components+=(media-worker)
validate_prepared_release "${selected_components[@]}"
validate_maintenance_artifact

validate_production_target
maintenance_read preflight >/dev/null
ensure_production_router_instance
configure_production_router
plan_json="$(maintenance_read plan)"
set +e
plan_requires_maintenance "$plan_json"
plan_status=$?
set -e
case "$plan_status" in
  0) ;;
  1)
    api_running="$(running_replicas "$api_service")"
    worker_running="$(running_replicas "$worker_service")"
    media_running="$(running_replicas "$media_service")"
    media_worker_running="$(running_replicas "$media_worker_service")"
    if [[ "$api_running" == "0" && "$worker_running" == "0" \
      && "$media_running" == "0" && "$media_worker_running" == "0" ]]; then
      api_has_history="$(service_has_deployment_history "$api_service")" || exit $?
      worker_has_history="$(service_has_deployment_history "$worker_service")" || exit $?
      media_had_history="$(service_has_deployment_history "$media_service")" || exit $?
      media_worker_had_history="$(service_has_deployment_history "$media_worker_service")" || exit $?
      if [[ "$api_has_history" != "$worker_has_history" ]]; then
        echo "Closed writers have inconsistent deployment history." >&2
        exit 1
      fi
      if [[ "$api_has_history" == "1" ]]; then
        echo "Writers are closed. Recover the recorded cutover ID and source SHA explicitly; a normal rerun cannot authorize reopening." >&2
        exit 1
      fi
      begin_cutover
      api_closed=1
      worker_closed=1
      media_closed=1
      media_worker_closed=1
      cutover_committed=1
      enter_public_gates
      deploy_and_reopen
      cutover_phase complete
      finish
    fi
    if [[ "$api_running" == "0" || "$worker_running" == "0" ]]; then
      echo "Metadata-writer replica state is inconsistent; refusing deployment." >&2
      exit 1
    fi
    for bootstrap_service in "$media_service" "$media_worker_service"; do
      bootstrap_running="$(running_replicas "$bootstrap_service")"
      if [[ "$bootstrap_running" == "0" ]]; then
        bootstrap_has_history="$(service_has_deployment_history "$bootstrap_service")" || exit $?
        if [[ "$bootstrap_has_history" == "1" ]]; then
          echo "Media writer $bootstrap_service is unexpectedly stopped; refusing deployment." >&2
          exit 1
        fi
      fi
    done
    if [[ "$deploy_router_requested" == "1" ]]; then
      activate_release git-router "$router_service"
      assert_router_topology
      promote_pending_evidence
    elif ! assert_router_topology || ! carried_service_is_healthy git-router "$router_service"; then
      echo "Production git-router must match the reviewed topology and durable deployment before deploying another backend service." >&2
      exit 1
    fi
    maintenance_read verify
    deploy_selected_releases
    finish
    ;;
  *)
    echo "Maintenance plan output was invalid; refusing deployment." >&2
    exit 1
    ;;
esac

if ! assert_router_topology || ! carried_service_is_healthy git-router "$router_service"; then
  echo "Maintenance requires the verified existing Git router until public gates are active." >&2
  exit 1
fi

if ! carried_service_is_healthy api "$api_service" \
  || ! carried_service_is_healthy run-worker "$worker_service"; then
  echo "Maintenance cutover requires healthy metadata-writer deployments before closing writers." >&2
  exit 1
fi

media_had_history="$(service_has_deployment_history "$media_service")" || exit $?
if [[ "$media_had_history" == "1" ]]; then
  carried_service_is_healthy media-api "$media_service" || {
    echo "Maintenance cutover requires a healthy media-api gateway before closing writers." >&2
    exit 1
  }
else
  media_closed=1
fi
media_worker_had_history="$(service_has_deployment_history "$media_worker_service")" || exit $?
if [[ "$media_worker_had_history" == "1" ]]; then
  carried_service_is_healthy media-worker "$media_worker_service" || {
    echo "Maintenance cutover requires a healthy media-api run-worker before closing writers." >&2
    exit 1
  }
else
  media_worker_closed=1
fi

fresh_plan="$(maintenance_read plan)"
if ! plans_have_same_ledger "$plan_json" "$fresh_plan"; then
  echo "Migration plan changed during release preparation; refusing writer closure." >&2
  exit 1
fi
plan_json="$fresh_plan"
begin_cutover
cutover_phase closing
quiesce_writers
cutover_phase closed
apply_cutover
cutover_phase complete
finish
