#!/usr/bin/env bash
set -euo pipefail

maintenance_binary="${SCOPE_MAINTENANCE_BINARY:-./target/release/scope-maintenance}"
environment="${SCOPE_RAILWAY_ENVIRONMENT_ID:?SCOPE_RAILWAY_ENVIRONMENT_ID is required}"
api_service="${SCOPE_RAILWAY_API_SERVICE_ID:?SCOPE_RAILWAY_API_SERVICE_ID is required}"
worker_service="${SCOPE_RAILWAY_WORKER_SERVICE_ID:?SCOPE_RAILWAY_WORKER_SERVICE_ID is required}"
cache_service="${SCOPE_RAILWAY_CACHE_SERVICE_ID:?SCOPE_RAILWAY_CACHE_SERVICE_ID is required}"
router_service="${SCOPE_RAILWAY_ROUTER_SERVICE_ID:?SCOPE_RAILWAY_ROUTER_SERVICE_ID is required}"
media_service="${SCOPE_RAILWAY_MEDIA_SERVICE_ID:?SCOPE_RAILWAY_MEDIA_SERVICE_ID is required}"
media_worker_service="${SCOPE_RAILWAY_MEDIA_WORKER_SERVICE_ID:?SCOPE_RAILWAY_MEDIA_WORKER_SERVICE_ID is required}"
router_group="${SCOPE_RAILWAY_ROUTER_GROUP_ID:?SCOPE_RAILWAY_ROUTER_GROUP_ID is required}"
database_service="${SCOPE_RAILWAY_DATABASE_SERVICE_ID:?SCOPE_RAILWAY_DATABASE_SERVICE_ID is required}"
api_region="${SCOPE_RAILWAY_API_REGION_ID:?SCOPE_RAILWAY_API_REGION_ID is required}"
worker_region="${SCOPE_RAILWAY_WORKER_REGION_ID:?SCOPE_RAILWAY_WORKER_REGION_ID is required}"
media_region="${SCOPE_RAILWAY_MEDIA_REGION_ID:?SCOPE_RAILWAY_MEDIA_REGION_ID is required}"
media_worker_image="${SCOPE_MEDIA_WORKER_IMAGE:-}"
deploy_cache_requested="${SCOPE_DEPLOY_CACHE:-1}"
deploy_worker_requested="${SCOPE_DEPLOY_WORKER:-1}"
deploy_router_requested="${SCOPE_DEPLOY_ROUTER:-1}"
deploy_api_requested="${SCOPE_DEPLOY_API:-1}"
deploy_media_requested="${SCOPE_DEPLOY_MEDIA:-1}"
deploy_media_worker_requested="${SCOPE_DEPLOY_MEDIA_WORKER:-1}"
successful_deployments="${SCOPE_SUCCESSFUL_DEPLOYMENTS:-}"
[[ -n "$successful_deployments" ]] || successful_deployments='{}'
deployment_evidence_path="${SCOPE_DEPLOYMENT_EVIDENCE_PATH:-}"
pending_evidence_path=""
if [[ -n "$deployment_evidence_path" ]]; then
  pending_evidence_path="${deployment_evidence_path}.pending.$$"
  rm -f -- "$pending_evidence_path"
fi

for deployment_flag in deploy_cache_requested deploy_worker_requested deploy_router_requested \
  deploy_api_requested deploy_media_requested deploy_media_worker_requested; do
  if [[ "${!deployment_flag}" != "0" && "${!deployment_flag}" != "1" ]]; then
    echo "${deployment_flag} must be 0 or 1." >&2
    exit 2
  fi
done
if [[ "$deploy_cache_requested" == "0" && "$deploy_worker_requested" == "0" \
  && "$deploy_router_requested" == "0" && "$deploy_api_requested" == "0" \
  && "$deploy_media_requested" == "0" && "$deploy_media_worker_requested" == "0" ]]; then
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

# Ordinary Railway CLI commands use the project token. Keep the workspace token isolated
# and expose it only to control-plane mutations sent through `railway api`.
railway_api_token="$RAILWAY_API_TOKEN"
unset RAILWAY_API_TOKEN

# One release policy owns operation limits. The CLI validates the same bounds.
policy_manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
web_service="$(jq -er .services.web.id "$policy_manifest")"
migration_lock_timeout_seconds="$(jq -er '.releasePolicy.migrationLockTimeoutSeconds | select(type == "number" and . > 0 and floor == .)' "$policy_manifest")"
migration_statement_timeout_seconds="$(jq -er '.releasePolicy.migrationStatementTimeoutSeconds | select(type == "number" and . > 0 and floor == .)' "$policy_manifest")"

railway_scope=(--project "$RAILWAY_PROJECT_ID" --environment "$environment")
cutover_committed=0
api_closed=0
worker_closed=0
cache_closed=0
media_closed=0
media_worker_closed=0
media_had_history=1
media_worker_had_history=1

maintenance() {
  # `railway run` executes on this CI host, so the database service's public proxy is required.
  # The child shell expands the Railway-injected database URL and command arguments.
  # shellcheck disable=SC2016
  railway run "${railway_scope[@]}" --service "$database_service" --no-local -- \
    sh -c 'DATABASE_URL="$DATABASE_PUBLIC_URL" exec "$@"' \
    scope-maintenance env \
      "SCOPE_MIGRATION_LOCK_TIMEOUT_SECONDS=$migration_lock_timeout_seconds" \
      "SCOPE_MIGRATION_STATEMENT_TIMEOUT_SECONDS=$migration_statement_timeout_seconds" \
      "$maintenance_binary" "$1"
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

run_api_maintenance() {
  local command="$1"
  local database_public_url maintenance_data_dir result
  database_public_url="$(
    railway variable list "${railway_scope[@]}" --service "$database_service" --json |
      jq -er '.DATABASE_PUBLIC_URL | strings | select(length > 0)'
  )"
  maintenance_data_dir="$(mktemp -d "${RUNNER_TEMP:-/tmp}/scope-repository-snapshot-backfill.XXXXXX")"
  result=0
  SCOPE_MAINTENANCE_DATABASE_URL="$database_public_url" \
    SCOPE_MAINTENANCE_DATA_DIR="$maintenance_data_dir" \
    railway run "${railway_scope[@]}" --service "$api_service" --no-local -- \
      sh -c 'DATABASE_URL="$SCOPE_MAINTENANCE_DATABASE_URL" SCOPE_DATA_DIR="$SCOPE_MAINTENANCE_DATA_DIR" exec "$@"' \
      scope-maintenance "$maintenance_binary" "$command" || result=$?
  rm -rf -- "$maintenance_data_dir"
  return "$result"
}

backfill_repository_snapshots() {
  run_api_maintenance backfill-workflow-catalogs
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
  PLAN_JSON="$1" node -e '
const plan = JSON.parse(process.env.PLAN_JSON || "{}");
if (!Array.isArray(plan.pending)) process.exit(2);
process.exit(plan.pending.length > 0 ? 0 : 1);
'
}

plan_is_exact() {
  PLAN_JSON="$1" node -e '
const plan = JSON.parse(process.env.PLAN_JSON || "{}");
process.exit(plan.exact === true && Array.isArray(plan.pending) && plan.pending.length === 0 ? 0 : 1);
'
}

plans_have_same_ledger() {
  BEFORE_PLAN_JSON="$1" AFTER_PLAN_JSON="$2" node -e '
const before = JSON.parse(process.env.BEFORE_PLAN_JSON || "{}");
const after = JSON.parse(process.env.AFTER_PLAN_JSON || "{}");
const ledger = (plan) => Array.isArray(plan.pending)
  ? plan.pending.map(({name}) => name)
  : null;
const beforeLedger = ledger(before);
const afterLedger = ledger(after);
process.exit(
  before.exact === after.exact &&
  Array.isArray(before.applied) && Array.isArray(after.applied) &&
  JSON.stringify(before.applied) === JSON.stringify(after.applied) &&
  beforeLedger !== null &&
  afterLedger !== null &&
  JSON.stringify(beforeLedger) === JSON.stringify(afterLedger)
    ? 0
    : 1,
);
'
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
    # A failed gate request can have succeeded at Railway. Identify the current
    # deployment before stopping anything, so cleanup never kills a serving gate.
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
  # Even a failed public gate must not prevent cleanup of other writers after a
  # partial activation. Each stop checks the exact current deployment identity.
  close_writer "$api_service" api_closed 1 || failed=1
  close_writer "$worker_service" worker_closed 0 || failed=1
  close_writer "$cache_service" cache_closed 0 || failed=1
  close_writer "$media_service" media_closed 1 || failed=1
  close_writer "$media_worker_service" media_worker_closed 0 || failed=1
  # Provider replica counts can be stale; the database fence proves writer closure.
  wait_for_writer_fence || failed=1
  return "$failed"
}

restore_old_release() {
  echo "Migration did not commit; restoring the previous metadata-writer deployments." >&2
  if [[ "$cache_closed" == "1" ]]; then
    restart_service "$cache_service" "$(require_successful_deployment_id cache)"
    cache_closed=0
  fi
  if [[ "$worker_closed" == "1" ]]; then
    restart_service "$worker_service" "$(require_successful_deployment_id run-worker)"
    worker_closed=0
  fi
  if [[ "$api_closed" == "1" ]]; then
    restart_service "$api_service" "$(require_successful_deployment_id api)"
    api_closed=0
  fi
  if [[ "$media_closed" == "1" ]]; then
    if [[ "$media_had_history" == "1" ]]; then
      restart_service "$media_service" "$(require_successful_deployment_id media-api)"
    fi
    media_closed=0
  fi
  if [[ "$media_worker_closed" == "1" ]]; then
    if [[ "$media_worker_had_history" == "1" ]]; then
      restart_service "$media_worker_service" "$(require_successful_deployment_id media-worker)"
    fi
    media_worker_closed=0
  fi
  mark_maintenance_end
}

deploy_release() {
  local component="$1"
  local service_name="$2"
  local verified_sha
  verified_sha="$(successful_deployment_field "$component" sourceSha)"
  SCOPE_DEPLOYMENT_COMPONENT="$component" \
    SCOPE_DEPLOYMENT_EVIDENCE_PATH="$pending_evidence_path" \
    SCOPE_DEFER_SERVICE_HEALTH=1 \
    SCOPE_VERIFIED_SUCCESSFUL_SHA="$verified_sha" \
    bash .github/scripts/deploy-railway.sh "$service_name"
}

pending_deployment_id() {
  local component="$1"
  [[ -n "$pending_evidence_path" && -s "$pending_evidence_path" ]] || return 0
  COMPONENT="$component" EVIDENCE_PATH="$pending_evidence_path" node -e '
const { readFileSync } = require("node:fs");
const records = readFileSync(process.env.EVIDENCE_PATH, "utf8")
  .trim().split(/\r?\n/).filter(Boolean).map(JSON.parse);
const evidence = records.findLast(({component}) => component === process.env.COMPONENT);
process.stdout.write(evidence?.evidenceId || "");
'
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
    SCOPE_DEPLOYMENT_EVIDENCE_PATH="$pending_evidence_path" \
    node .github/scripts/deploy-railway-image.mjs "$service_name" "$image"
  expected_deployment_id="$(pending_deployment_id "$component")"
  [[ -n "$expected_deployment_id" ]] || {
    echo "Pinned image deployment produced no Railway evidence for $component." >&2
    return 1
  }
  wait_for_service_health "$service_name" "$expected_deployment_id"
}

promote_pending_evidence() {
  [[ -n "$deployment_evidence_path" && -s "$pending_evidence_path" ]] || return 0
  FINAL_EVIDENCE_PATH="$deployment_evidence_path" \
    PENDING_EVIDENCE_PATH="$pending_evidence_path" \
    node -e '
const { appendFileSync, readFileSync, unlinkSync } = require("node:fs");
appendFileSync(process.env.FINAL_EVIDENCE_PATH, readFileSync(process.env.PENDING_EVIDENCE_PATH));
unlinkSync(process.env.PENDING_EVIDENCE_PATH);
'
}

discard_pending_evidence() {
  [[ -z "$pending_evidence_path" ]] || rm -f -- "$pending_evidence_path"
}

deploy_selected_releases() {
  if [[ "$deploy_cache_requested" == "1" ]]; then
    activate_release cache "$cache_service"
    promote_pending_evidence
  fi
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
  cutover_phase backfills
  backfill_repository_snapshots
  # Activation can create a live replacement before its response fails. Mark it potentially open
  # first so the failure handler stops whichever deployment the provider currently reports.
  cutover_phase activating-cache
  cache_closed=0
  activate_release cache "$cache_service"
  cutover_phase activating-worker
  worker_closed=0
  activate_release run-worker "$worker_service"
  cutover_phase activating-media
  media_closed=0
  activate_release media-api "$media_service"
  cutover_phase activating-media-worker
  media_worker_closed=0
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
  mark_maintenance_end
  # Every database writer forms one cutover. Publish their evidence only after all writers are healthy
  # so the durable ledger cannot claim a deployment that the failure trap subsequently closes.
  promote_pending_evidence
}

leave_failure_state() {
  local exit_status="$1"
  trap - EXIT
  if [[ "$exit_status" -ne 0 ]]; then
    if [[ "$cutover_committed" == "0" \
      && ( "$api_closed" == "1" || "$worker_closed" == "1" || "$cache_closed" == "1" \
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
  trap - EXIT
  discard_pending_evidence
  exit 0
fi
journal cutover-guard
selected_components=()
[[ "$deploy_api_requested" == "0" ]] || selected_components+=(api)
[[ "$deploy_worker_requested" == "0" ]] || selected_components+=(run-worker)
[[ "$deploy_cache_requested" == "0" ]] || selected_components+=(cache)
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
    cache_running="$(running_replicas "$cache_service")"
    media_running="$(running_replicas "$media_service")"
    media_worker_running="$(running_replicas "$media_worker_service")"
    if [[ "$api_running" == "0" && "$worker_running" == "0" && "$cache_running" == "0" \
      && "$media_running" == "0" && "$media_worker_running" == "0" ]]; then
      api_has_history=0
      worker_has_history=0
      cache_has_history=0
      service_has_deployment_history "$api_service" && api_has_history=1
      service_has_deployment_history "$worker_service" && worker_has_history=1
      service_has_deployment_history "$cache_service" && cache_has_history=1
      media_had_history=0
      media_worker_had_history=0
      service_has_deployment_history "$media_service" && media_had_history=1
      service_has_deployment_history "$media_worker_service" && media_worker_had_history=1
      if [[ "$api_has_history" != "$worker_has_history" ]] \
        || [[ "$api_has_history" != "$cache_has_history" ]]; then
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
      cache_closed=1
      media_closed=1
      media_worker_closed=1
      cutover_committed=1
      enter_public_gates
      deploy_and_reopen
      cutover_phase complete
      trap - EXIT
      discard_pending_evidence
      exit 0
    fi
    if [[ "$api_running" == "0" || "$worker_running" == "0" || "$cache_running" == "0" ]]; then
      echo "Metadata-writer replica state is inconsistent; refusing deployment." >&2
      exit 1
    fi
    for bootstrap_service in "$media_service" "$media_worker_service"; do
      bootstrap_running="$(running_replicas "$bootstrap_service")"
      if [[ "$bootstrap_running" == "0" ]] && service_has_deployment_history "$bootstrap_service"; then
        echo "Media writer $bootstrap_service is unexpectedly stopped; refusing deployment." >&2
        exit 1
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
    trap - EXIT
    discard_pending_evidence
    exit 0
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
  || ! carried_service_is_healthy run-worker "$worker_service" \
  || ! carried_service_is_healthy cache "$cache_service"; then
  echo "Maintenance cutover requires healthy metadata-writer deployments before closing writers." >&2
  exit 1
fi

if service_has_deployment_history "$media_service"; then
  carried_service_is_healthy media-api "$media_service" || {
    echo "Maintenance cutover requires a healthy media-api gateway before closing writers." >&2
    exit 1
  }
else
  media_had_history=0
  media_closed=1
fi
if service_has_deployment_history "$media_worker_service"; then
  carried_service_is_healthy media-worker "$media_worker_service" || {
    echo "Maintenance cutover requires a healthy media-api run-worker before closing writers." >&2
    exit 1
  }
else
  media_worker_had_history=0
  media_worker_closed=1
fi

# Preparation may take minutes. Read the plan again immediately before recording closure intent.
# Refuse a changed ledger so a new invocation can prepare against the actual database state.
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
trap - EXIT
discard_pending_evidence
