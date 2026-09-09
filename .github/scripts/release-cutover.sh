#!/usr/bin/env bash
# Durable cutover intent is persisted before each irreversible phase.
cutover_id="${SCOPE_RELEASE_CUTOVER_ID:-}"
prepared_release_path="${SCOPE_PREPARED_RELEASE_PATH:?SCOPE_PREPARED_RELEASE_PATH is required}"
source_sha="${SCOPE_DEPLOYMENT_SOURCE_SHA:-${GITHUB_SHA:?GITHUB_SHA is required}}"
phase_started=$SECONDS
current_phase=prepare

journal() {
  node .github/scripts/production-deployment-progress.mjs "$@"
}

cutover_phase() {
  local next="$1" elapsed=$((SECONDS - phase_started))
  printf 'Release phase %s: %ss\n' "$current_phase" "$elapsed"
  if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    printf '| %s | %s |\n' "$current_phase" "$elapsed" >> "$GITHUB_STEP_SUMMARY"
  fi
  [[ -z "$cutover_id" ]] || journal cutover-phase --id "$cutover_id" --phase "$next"
  current_phase="$next"
  phase_started=$SECONDS
}

validate_prepared_release() {
  node .github/scripts/railway-artifact.mjs validate "$prepared_release_path" "$source_sha" "$@" >/dev/null
}

validate_maintenance_artifact() {
  node --input-type=module -e '
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
const prepared = JSON.parse(readFileSync(process.argv[1], "utf8"));
const digest = createHash("sha256").update(readFileSync(process.argv[2])).digest("hex");
if (prepared.maintenanceSha256 !== digest) throw new Error("Maintenance binary does not match the prepared release");
' "$prepared_release_path" "$maintenance_binary"
}

begin_cutover() {
  local temporary
  if plan_requires_maintenance "$plan_json" && [[ ! "${SCOPE_MAINTENANCE_OUTAGE_BUDGET_MS:-}" =~ ^[1-9][0-9]*$ ]]; then
    echo "A positive SCOPE_MAINTENANCE_OUTAGE_BUDGET_MS must be approved from staging measurements before writer closure." >&2
    return 1
  fi
  validate_prepared_release api worker cache router media mediaWorker
  validate_maintenance_artifact
  temporary="$(mktemp -d)"
  printf '%s\n' "$plan_json" > "$temporary/baseline.json"
  printf '%s\n' "$successful_deployments" > "$temporary/previous.json"
  cutover_id="$(journal cutover-begin --manifest "$prepared_release_path" \
    --baseline "$temporary/baseline.json" --previous "$temporary/previous.json")"
  rm -rf -- "$temporary"
  echo "Durable release cutover: $cutover_id"
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    printf 'cutover_id=%s\n' "$cutover_id" >> "$GITHUB_OUTPUT"
  fi
}

recover_cutover() {
  local record pinned_manifest recovery_plan
  record="$(journal cutover-read --id "$cutover_id" --source-sha "$source_sha")"
  pinned_manifest="$(jq -cS .prepared <<< "$record")"
  if [[ "$(jq -cS . "$prepared_release_path")" != "$pinned_manifest" ]]; then
    echo "Recovery artifacts do not match the durable cutover manifest." >&2
    return 1
  fi
  validate_prepared_release api worker cache router media mediaWorker
  validate_maintenance_artifact
  successful_deployments="$(jq -c .previous <<< "$record")"
  plan_json="$(jq -c .baseline <<< "$record")"
  # A runner may disappear between any provider mutation and its response. Stop every current
  # writer again, including a partially activated release, before interpreting the ledger.
  cutover_committed=1
  cutover_phase reclosing
  quiesce_writers
  recovery_plan="$(maintenance_read plan)"
  if plan_is_exact "$recovery_plan"; then
    deploy_and_reopen
  elif plans_have_same_ledger "$plan_json" "$recovery_plan"; then
    apply_cutover
  else
    echo "Recovery ledger matches neither the pinned baseline nor the prepared release; writers remain closed." >&2
    return 1
  fi
  cutover_phase complete
}

apply_cutover() {
  cutover_phase pre-migration
  maintenance validate-workflow-catalogs
  # Persist applying before invoking the transaction. SIGKILL must never erase its uncertainty.
  cutover_phase applying
  cutover_committed=1
  if ! maintenance apply; then
    recovery_plan="$(maintenance_read plan || true)"
    if [[ -n "$recovery_plan" ]] && plans_have_same_ledger "$plan_json" "$recovery_plan"; then
      cutover_committed=0
    fi
    return 1
  fi
  cutover_phase committed
  deploy_and_reopen
}

mark_maintenance_start() {
  if [[ -n "${SCOPE_RELEASE_MAINTENANCE_START_FILE:-}" && ! -e "$SCOPE_RELEASE_MAINTENANCE_START_FILE" ]]; then
    date +%s%3N > "$SCOPE_RELEASE_MAINTENANCE_START_FILE"
  fi
}

mark_maintenance_end() {
  if [[ -n "${SCOPE_RELEASE_MAINTENANCE_END_FILE:-}" ]]; then
    date +%s%3N > "$SCOPE_RELEASE_MAINTENANCE_END_FILE"
  fi
}
