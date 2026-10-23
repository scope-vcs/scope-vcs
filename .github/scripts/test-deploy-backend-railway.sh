#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT
mkdir -p "$test_dir/bin"
cat > "$test_dir/bin/sleep" <<'FAKE'
#!/usr/bin/env bash
exit 0
FAKE
chmod +x "$test_dir/bin/sleep"

cat > "$test_dir/services.json" <<'JSON'
{"railway":{"projectId":"11111111-1111-4111-8111-111111111111","maintenanceServiceId":"33333333-3333-4333-8333-333333333333"},"environments":{"production":{"environmentId":"22222222-2222-4222-8222-222222222222"},"staging":{"environmentId":"44444444-4444-4444-8444-444444444444"}},"releasePolicy":{"maintenanceEnabled":true,"writerDrainTimeoutSeconds":120,"migrationLockTimeoutSeconds":120,"migrationStatementTimeoutSeconds":3600},"services":{"api":{"id":"scope-api","deployment":{"runtimeConfig":"api/railway.json","verifyTransitionConfig":true}},"run-worker":{"id":"scope-worker","deployment":{"runtimeConfig":"worker/railway.json","verifyTransitionConfig":true}},"git-router":{"id":"scope-repo-router","deployment":{"runtimeConfig":"repo-router/railway.json","verifyTransitionConfig":true}},"media-api":{"id":"scope-media","deployment":{"runtimeConfig":"media-service/railway.json","verifyTransitionConfig":true}},"media-worker":{"id":"scope-media-worker","deployment":{"runtimeConfig":"media-worker/railway.json","verifyTransitionConfig":true}},"web":{"id":"scope-web","deployment":{"runtimeConfig":"web/railway.json","verifyTransitionConfig":true}}}}
JSON

cat > "$test_dir/github-fetch.cjs" <<'FAKE'
const fs = require("node:fs");
const original = global.fetch;
global.fetch = async (url, options = {}) => {
  if (!String(url).startsWith("https://api.github.com/repos/test/repo/")) return original(url, options);
  const path = new URL(url).pathname.replace("/repos/test/repo", "");
  const file = `${process.env.FAKE_RAILWAY_STATE}/journal.json`;
  const state = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : { deployments: [], statuses: {} };
  const sourceSha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const apiResponse = result => new Response(JSON.stringify(result), { status: 200 });
  if (path === "/actions/runs/123") return apiResponse({
    id:123, path:".github/workflows/release.yml", event:"schedule",
    head_branch:"main", head_sha:sourceSha, conclusion:"cancelled",
    repository:{id:1,full_name:"test/repo"}, head_repository:{id:1,full_name:"test/repo"},
  });
  if (path === "/branches/main") return apiResponse({name:"main",commit:{sha:sourceSha}});
  if (path === `/compare/${sourceSha}...${sourceSha}`) return apiResponse({
    status:"identical",base_commit:{sha:sourceSha},merge_base_commit:{sha:sourceSha},
  });
  if (path === "/actions/runs/123/jobs") return apiResponse({jobs:[{
    id:456,run_id:123,head_sha:sourceSha,name:"Prepare Railway artifacts / prepare",
    status:"completed",conclusion:"success",
    steps:[{name:"Prepare immutable release images",conclusion:"success"}],
  }]});
  const body = options.body ? JSON.parse(options.body) : null;
  let result;
  if (path === "/deployments" && body) {
    result = { ...body, sha: body.ref, id: state.deployments.length + 1 };
    state.deployments.unshift(result);
    state.statuses[result.id] = [];
  } else if (path === "/deployments") result = state.deployments;
  else {
    const [, id, suffix] = /^\/deployments\/(\d+)(.*)$/.exec(path);
    if (!suffix) result = state.deployments.find(d => String(d.id) === id);
    else {
      if (body) state.statuses[id].unshift({ ...body, created_at: new Date().toISOString() });
      result = state.statuses[id];
    }
  }
  fs.writeFileSync(file, JSON.stringify(state));
  return new Response(JSON.stringify(result), { status: 200 });
};
FAKE

cat > "$test_dir/maintenance" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$SCOPE_MAINTENANCE_BINARY $*" >> "$FAKE_RAILWAY_TRACE"
[[ "${DATABASE_URL:-}" == "postgres://scope-postgres.railway.internal/scope" ]]

case "${1:-}" in
  preflight)
    [[ "${FAKE_SCHEMA_DRIFT:-0}" != "1" ]]
    ;;
  plan)
    if [[ "${FAKE_FAIL_FIRST_PLAN:-0}" == "1" && ! -f "$FAKE_RAILWAY_STATE/first-plan-failed" ]]; then
      touch "$FAKE_RAILWAY_STATE/first-plan-failed"
      exit 1
    fi
    if [[ "${FAKE_FAIL_RECOVERY_PLAN:-0}" == "1" && -f "$FAKE_RAILWAY_STATE/apply-attempted" ]]; then
      exit 1
    fi
    if [[ "${FAKE_CHANGE_PLAN:-0}" == "1" && -f "$FAKE_RAILWAY_STATE/plan-read" ]]; then
      echo '{"exact":false,"applied":[],"pending":[{"name":"changed"}]}'
      exit 0
    fi
    touch "$FAKE_RAILWAY_STATE/plan-read"
    if [[ -f "$FAKE_RAILWAY_STATE/exact" ]]; then
      echo '{"exact":true,"applied":["m0084_current_schema_baseline"],"pending":[]}'
    else
      echo '{"exact":false,"applied":[],"pending":[{"name":"m0084_current_schema_baseline"}]}'
    fi
    ;;
  apply)
    touch "$FAKE_RAILWAY_STATE/apply-attempted"
    [[ "${FAKE_FAIL_APPLY:-0}" == "rollback" ]] && exit 1
    touch "$FAKE_RAILWAY_STATE/exact"
    if [[ "${FAKE_KILL_CUTOVER_PHASE:-}" == "apply" ]]; then
      kill -KILL "$FAKE_CUTOVER_PID"
      exit 137
    fi
    [[ "${FAKE_FAIL_APPLY:-0}" == "committed-error" ]] && exit 1
    echo '{"exact":true,"migration":"applied"}'
    ;;
  fence)
    if [[ "${FAKE_STALE_STOP_STATUS:-0}" == "1" ]]; then
      [[ -f "$FAKE_RAILWAY_STATE/stop-requested-scope-api" ]]
      [[ -f "$FAKE_RAILWAY_STATE/stop-requested-scope-worker" ]]
    fi
    if [[ "${FAKE_STUCK_FENCE:-0}" == "1" && ! -f "$FAKE_RAILWAY_STATE/writers-drained" ]]; then
      exit 1
    fi
    echo '{"available":true}'
    ;;
  drain-writers)
    touch "$FAKE_RAILWAY_STATE/writers-drained"
    echo '{"terminated":1}'
    ;;
  validate-workflow-catalogs)
    [[ "${FAKE_FAIL_WORKFLOW_VALIDATION:-0}" != "1" ]]
    echo '{"workflowCatalogsValidated":1}'
    ;;
  verify)
    [[ -f "$FAKE_RAILWAY_STATE/exact" ]]
    echo '{"exact":true}'
    ;;
  *)
    exit 2
    ;;
esac
FAKE
chmod +x "$test_dir/maintenance"

source "$root/.github/scripts/backend-cutover-test-provider.sh"

sha=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
receipt() { printf '"%s":{"sourceSha":"%s","provider":"railway","evidenceId":"%s"}' "$1" "$sha" "$2"; }
router_receipt="$(receipt git-router old-scope-repo-router)"
default_receipts="{$(receipt api old-scope-api),$(receipt run-worker old-scope-worker),$(receipt media-api old-scope-media),\"media-worker\":{\"sourceSha\":\"$sha\",\"provider\":\"railway\",\"evidenceId\":\"old-scope-media-worker\",\"artifactDigest\":\"sha256:${sha}aaaaaaaaaaaaaaaaaaaaaaaa\"},$router_receipt}"
m="$test_dir/maintenance"
up="up $test_dir"

run_cutover() {
  local expected="$1" name="$2"
  local state="$test_dir/$name-state" trace="$test_dir/$name-trace" result
  local successful_deployments="${SCOPE_SUCCESSFUL_DEPLOYMENTS:-$default_receipts}"
  mkdir -p "$state"
  [[ -z "${FAKE_STOPPED_MEDIA_SERVICE:-}" ]] || touch "$state/stopped-$FAKE_STOPPED_MEDIA_SERVICE"
  [[ "${FAKE_INITIAL_EXACT:-0}" != 1 ]] || touch "$state/exact"
  [[ "${FAKE_INITIAL_CLOSED:-0}" != 1 ]] || touch "$state"/stopped-scope-{api,worker,media,media-worker}
  if [[ "${FAKE_NO_HISTORY:-0}" == 1 ]]; then
    successful_deployments="{$router_receipt}"
    touch "$state"/no-history-scope-{api,worker,media,media-worker}
  fi
  [[ "${FAKE_ROUTER_CONFIGURED:-1}" != 0 ]] || touch "$state/no-history-scope-repo-router"
  : > "$trace"
  node -e '
const fs = require("node:fs");
const sourceSha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const names = { api:"scope-api","run-worker":"scope-worker","git-router":"scope-repo-router","media-api":"scope-media","media-worker":"scope-media-worker",web:"scope-web" };
const components = Object.fromEntries(Object.entries(names).map(([component,serviceId]) => [component,{
serviceId,sourceSha,image:component === "media-worker"
  ? `ghcr.io/scope-vcs/scope-media-worker@sha256:${"a".repeat(64)}`
  : `ghcr.io/test/repo/railway-private-${({"run-worker":"worker","git-router":"router","media-api":"media"})[component] || component}@sha256:${"b".repeat(64)}`}]))
delete components[process.argv[3]];
const maintenanceSha256 = require("node:crypto").createHash("sha256").update(fs.readFileSync(process.argv[2])).digest("hex");
fs.writeFileSync(process.argv[1], JSON.stringify({schemaVersion:1,sourceSha,components,maintenanceSha256,preparationRunId:"123"}));
' "$test_dir/$name-prepared.json" "$m" "${FAKE_MISSING_PREPARED_COMPONENT:-}"
  set +e
  PATH="$test_dir/bin:$PATH" \
    SCOPE_RAILWAY_READ_RETRY_DELAY_MS=0 \
    NODE_OPTIONS="--require=$test_dir/github-fetch.cjs" \
    GITHUB_TOKEN="test-token" \
    GITHUB_REPOSITORY="test/repo" \
    SCOPE_PREPARED_RELEASE_PATH="$test_dir/$name-prepared.json" \
    SCOPE_RELEASE_CUTOVER_ID="$([[ "${FAKE_RECOVER_CLOSED_CUTOVER:-0}" != 1 ]] || echo 1)" \
    FAKE_RAILWAY_STATE="$state" \
    FAKE_RAILWAY_TRACE="$trace" \
    FAKE_UPLOAD_ROOT="$test_dir" \
    SCOPE_DEPLOYMENT_MANIFEST="${FAKE_MANIFEST:-$test_dir/services.json}" \
    RAILWAY_PROJECT_ID="11111111-1111-4111-8111-111111111111" \
    RAILWAY_API_TOKEN="token-graphql" \
    RAILWAY_TOKEN="token-project" \
    SCOPE_RAILWAY_ENVIRONMENT_ID="22222222-2222-4222-8222-222222222222" \
    SCOPE_RAILWAY_DATABASE_SERVICE_ID="scope-postgres" \
    SCOPE_RAILWAY_API_SERVICE_ID="scope-api" \
    SCOPE_RAILWAY_WORKER_SERVICE_ID="scope-worker" \
    SCOPE_RAILWAY_ROUTER_SERVICE_ID="scope-repo-router" \
    SCOPE_RAILWAY_MEDIA_SERVICE_ID="scope-media" \
    SCOPE_RAILWAY_MEDIA_WORKER_SERVICE_ID="scope-media-worker" \
    SCOPE_RAILWAY_ROUTER_GROUP_ID="runtime-group" \
    SCOPE_RAILWAY_API_REGION_ID="us-east4-eqdc4a" \
    SCOPE_RAILWAY_WORKER_REGION_ID="us-east4-eqdc4a" \
    SCOPE_RAILWAY_MEDIA_REGION_ID="us-east4-eqdc4a" \
    SCOPE_MEDIA_WORKER_IMAGE="ghcr.io/scope-vcs/scope-media-worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
    SCOPE_DEPLOY_WORKER="${SCOPE_DEPLOY_WORKER:-1}" \
    SCOPE_DEPLOY_ROUTER="${SCOPE_DEPLOY_ROUTER:-0}" \
    SCOPE_DEPLOY_API="${SCOPE_DEPLOY_API:-1}" \
    SCOPE_DEPLOY_MEDIA=0 \
    SCOPE_DEPLOY_MEDIA_WORKER=0 \
    SCOPE_SUCCESSFUL_DEPLOYMENTS="$successful_deployments" \
    SCOPE_DEPLOYMENT_EVIDENCE_PATH="$test_dir/$name-evidence.jsonl" \
    SCOPE_SERVICE_HEALTH_TIMEOUT_SECONDS=0 \
    SCOPE_SERVICE_HEALTH_POLL_SECONDS=0 \
    SCOPE_PREDECESSOR_TEARDOWN_POLL_SECONDS=0 \
    GITHUB_SHA="$sha" \
    SCOPE_WRITER_FENCE_GRACE_SECONDS="0" \
    SCOPE_MAINTENANCE_BINARY="$m" \
    bash -c 'export FAKE_CUTOVER_PID=$$; exec bash "$@"' _ \
      "$root/.github/scripts/deploy-backend-railway.sh"
  result=$?
  set -e
  if { [[ "$expected" == pass ]] && [[ "$result" != 0 ]]; } || { [[ "$expected" == fail ]] && [[ "$result" == 0 ]]; }; then
    echo "$name was expected to $expected but exited $result" >&2
    exit 1
  fi
}

refute() {
  if grep -E -- "$2" "$test_dir/$1-trace"; then
    echo "$1: $3" >&2
    exit 1
  fi
}

assert_count() {
  [[ "$(grep -F -c "${@:4}" -- "$3" "$test_dir/$1-trace")" == "$2" ]] || {
    echo "$1: expected $2 occurrences of '$3'" >&2
    exit 1
  }
}

assert_evidence_components() {
  local actual=()
  [[ ! -f "$test_dir/$1-evidence.jsonl" ]] || mapfile -t actual < <(jq -r .component "$test_dir/$1-evidence.jsonl" | sort)
  [[ "$(IFS=,; echo "${actual[*]}")" == "$(tr , '\n' <<< "$2" | sort | paste -sd, -)" ]] || {
    echo "$1: expected evidence $2, got ${actual[*]}" >&2
    exit 1
  }
}

assert_in_order() {
  local trace="$test_dir/$1-trace" previous=0 pattern line
  shift
  for pattern in "$@"; do
    line="$(grep -n -F -- "$pattern" "$trace" | cut -d: -f1 | awk -v previous="$previous" '$1 > previous { print; exit }')"
    [[ -n "$line" ]] || {
      echo "missing or out-of-order '$pattern' in $trace" >&2
      return 1
    }
    previous="$line"
  done
}

no_closure='graphql stop|maintenance apply'
no_mutation='graphql (stop|restart)|gate (enter|reclose)|up |maintenance apply'
no_router_mutation='graphql (create-instance|configure-scale|stop|restart)|gate (enter|reclose|restore)|domain --|variable set|up |maintenance (apply|drain-writers|backfill-|cleanup-|scrub-)'
scope_args="--project 11111111-1111-4111-8111-111111111111 --environment 22222222-2222-4222-8222-222222222222"

disabled_maintenance() {
  jq '.releasePolicy.maintenanceEnabled = false' "$test_dir/services.json" > "$test_dir/disabled-services.json"
  FAKE_MANIFEST="$test_dir/disabled-services.json" run_cutover fail disabled-maintenance
  refute disabled-maintenance "$no_closure" "disabled maintenance policy must prevent closure"
}

missing_prepared_worker() {
  FAKE_MISSING_PREPARED_COMPONENT=run-worker run_cutover fail missing-prepared-worker
  refute missing-prepared-worker "$no_closure" "missing prepared artifacts must prevent closure"
}

schema_drift() {
  FAKE_SCHEMA_DRIFT=1 run_cutover fail schema-drift
  refute schema-drift "$no_closure" "baseline schema drift must prevent writer closure"
}

changed_plan() {
  FAKE_CHANGE_PLAN=1 run_cutover fail changed-plan
  refute changed-plan "$no_closure" "changed preclosure plan must prevent closure"
}

history_failure_media() {
  FAKE_HISTORY_FAILURE_SERVICE=scope-media run_cutover fail history-failure-scope-media
  refute history-failure-scope-media "$no_mutation" "failed deployment inventory must not authorize closure or activation"
}

history_failure_bootstrap() {
  FAKE_INITIAL_EXACT=1 FAKE_INITIAL_CLOSED=1 FAKE_NO_HISTORY=1 \
    FAKE_HISTORY_FAILURE_SERVICE=scope-api run_cutover fail history-failure-bootstrap
  refute history-failure-bootstrap "$no_mutation" "failed deployment inventory must not authorize bootstrap"
}

history_failure_stopped_media() {
  FAKE_INITIAL_EXACT=1 FAKE_STOPPED_MEDIA_SERVICE=scope-media \
    FAKE_HISTORY_FAILURE_SERVICE=scope-media run_cutover fail history-failure-stopped-media
  refute history-failure-stopped-media "$no_mutation" "failed deployment inventory must not authorize bootstrap"
}

success() {
  run_cutover pass success
  assert_evidence_components success run-worker,media-api,media-worker,api,git-router,web
  refute success "--path-as-root" "maintenance activation must not upload a build context"
  assert_in_order success "$m plan" "graphql stop scope-api old-scope-api" "graphql stop scope-worker old-scope-worker" \
    "$m fence" "$m validate-workflow-catalogs" "$m apply" "$m verify" "$up/media-api" "$up/run-worker" "$up/api"
}

parallel_activation() {
  FAKE_REQUIRE_PARALLEL_PAIR=1 run_cutover pass parallel-activation
  assert_evidence_components parallel-activation run-worker,media-api,media-worker,api,git-router,web
  assert_in_order parallel-activation "$m verify" "$up/media-api" "$up/run-worker" "$up/api" "$up/git-router"
  assert_in_order parallel-activation "$up/media-api" "$up/media-worker" "$up/api" "$up/web"
}

delayed_teardown() {
  FAKE_DELAY_REMOVE_UNTIL_ROUTER=1 run_cutover pass delayed-teardown
  assert_in_order delayed-teardown "$up/media-api" "$up/run-worker" "$up/api" "$up/git-router"
}

migration_router_selected() {
  SCOPE_DEPLOY_ROUTER=1 run_cutover pass migration-router-selected
  assert_count migration-router-selected 1 "$up/git-router "
  assert_in_order migration-router-selected "$m apply" "$up/api" "$up/git-router" "$up/web"
}

stale_stop_status() {
  FAKE_STALE_STOP_STATUS=1 run_cutover pass stale-stop-status
  assert_in_order stale-stop-status "graphql stop scope-api old-scope-api" "graphql stop scope-worker old-scope-worker" "$m fence" "$m apply"
}

draining_writer() {
  FAKE_STUCK_FENCE=1 run_cutover pass draining-writer
  assert_count draining-writer 2 "$m fence" -x
  assert_in_order draining-writer "graphql stop scope-worker old-scope-worker" "$m fence" "$m drain-writers" "$m fence" \
    "$m validate-workflow-catalogs" "$m apply"
}

invalid_workflow_catalog() {
  FAKE_FAIL_WORKFLOW_VALIDATION=1 run_cutover fail invalid-workflow-catalog
  assert_in_order invalid-workflow-catalog "graphql stop scope-api old-scope-api" "graphql stop scope-worker old-scope-worker" \
    "$m fence" "$m validate-workflow-catalogs"
  refute invalid-workflow-catalog "graphql restart " "gate replacement requires pinned forward recovery even before schema apply"
  refute invalid-workflow-catalog "$m apply" "invalid workflow catalogs must fail before migration"
}

degraded() {
  FAKE_DEGRADED_SERVICE=scope-worker run_cutover fail degraded
  refute degraded "graphql (stop|restart) |$m apply" "maintenance must not start from a degraded service"
}

degraded_media() {
  FAKE_DEGRADED_SERVICE=scope-media run_cutover fail degraded-media
  refute degraded-media "graphql (stop|restart) |$m apply" "maintenance must not start with a degraded media writer"
}

wrong_api_region() {
  FAKE_API_REGION=us-west2 run_cutover fail wrong-api-region
  refute wrong-api-region "graphql (stop|restart) |$m apply" "region drift must fail before maintenance starts"
}

degraded_during_shutdown() {
  FAKE_DEGRADE_WORKER_AFTER_API_STOP=1 run_cutover fail degraded-during-shutdown
  assert_in_order degraded-during-shutdown "graphql stop scope-api old-scope-api" "graphql stop scope-worker old-scope-worker" \
    "$m fence" "$m apply" "$up/run-worker" "graphql stop scope-worker new-scope-worker"
}

crashed_worker() {
  FAKE_CRASH_UP_SERVICE=scope-worker run_cutover fail crashed-worker
  assert_evidence_components crashed-worker ""
  [[ -f "$test_dir/crashed-worker-state/stopped-scope-worker" ]]
  assert_in_order crashed-worker "$m apply" "$up/run-worker" "graphql stop scope-media new-scope-media"
}

evidence_promotion() {
  mkdir "$test_dir/evidence-promotion-evidence.jsonl"
  run_cutover fail evidence-promotion 2> "$test_dir/evidence-promotion-stderr"
  [[ -f "$test_dir/evidence-promotion-state/up-scope-media" ]]
  [[ -f "$test_dir/evidence-promotion-state/stopped-scope-media" ]]
  assert_in_order evidence-promotion "$m apply" "$up/media-api" "graphql stop scope-media new-scope-media" "$m fence"
  assert_count evidence-promotion 1 "graphql stop scope-media new-scope-media"
  assert_in_order evidence-promotion "$up/run-worker" "$up/api" "graphql stop scope-api new-scope-api"
  if compgen -G "$test_dir/evidence-promotion-evidence.jsonl.pending.*" > /dev/null; then
    echo "failed evidence promotion must discard pending evidence" >&2
    exit 1
  fi
  [[ -z "$(find "$test_dir/evidence-promotion-evidence.jsonl" -mindepth 1 -print -quit)" ]]
  grep -F "Cutover requires forward recovery; writers are closed. Rerun this workflow to finish the pinned deployment." \
    "$test_dir/evidence-promotion-stderr"
  if grep -F "Failed to re-close metadata writers after the cutover." "$test_dir/evidence-promotion-stderr"; then
    echo "evidence promotion recovery must report successful writer closure" >&2
    exit 1
  fi
}

rollback() {
  FAKE_FAIL_APPLY=rollback run_cutover fail rollback
  assert_in_order rollback "$m apply" "$m plan"
  refute rollback 'graphql restart|up ' "removed predecessors must not be restarted after gate replacement"
  FAKE_RECOVER_CLOSED_CUTOVER=1 run_cutover pass rollback
  assert_in_order rollback "$m plan" "$m apply" "$m verify" "$up/media-api" "$up/run-worker" "$up/api"
}

unknown() {
  FAKE_FAIL_APPLY=committed-error FAKE_FAIL_RECOVERY_PLAN=1 run_cutover fail unknown
  refute unknown "graphql restart " "unknown migration state must not restore old deployments"
  refute unknown "$up/api" "unknown migration state must not deploy new binaries"
}

rolling() {
  FAKE_INITIAL_EXACT=1 run_cutover pass rolling
  assert_in_order rolling "$m plan" "$m verify" "$up/run-worker" "$up/api"
  refute rolling "graphql (stop|restart) |maintenance (backfill-|cleanup-)" \
    "exact-schema deployment must stay on the rolling path without repeating migration data work"
}

rolling_worker() {
  FAKE_INITIAL_EXACT=1 SCOPE_DEPLOY_API=0 run_cutover pass rolling-worker
  assert_evidence_components rolling-worker run-worker
  assert_in_order rolling-worker "$m verify" "$up/run-worker"
  refute rolling-worker "$up/api" "worker-only deployment must not deploy the API"
}

carried_worker_drift() {
  FAKE_INITIAL_EXACT=1 SCOPE_DEPLOY_API=0 FAKE_SKIP_UP_SERVICE=scope-worker \
    SCOPE_SUCCESSFUL_DEPLOYMENTS="{$(receipt run-worker new-scope-worker),$router_receipt}" \
    run_cutover fail carried-worker-drift
  assert_evidence_components carried-worker-drift ""
  refute carried-worker-drift "graphql restart scope-worker" \
    "a skipped deploy with drifted active identity must not restart the carried deployment"
}

rolling_worker_unhealthy() {
  FAKE_INITIAL_EXACT=1 SCOPE_DEPLOY_API=0 FAKE_UNHEALTHY_AFTER_UP_SERVICE=scope-worker \
    run_cutover fail rolling-worker-unhealthy
  assert_evidence_components rolling-worker-unhealthy ""
}

router_bootstrap() {
  FAKE_INITIAL_EXACT=1 SCOPE_DEPLOY_WORKER=0 SCOPE_DEPLOY_ROUTER=1 FAKE_ROUTER_CONFIGURED=0 FAKE_ROUTER_INSTANCE_EXISTS=0 \
    run_cutover pass router-bootstrap
  assert_evidence_components router-bootstrap git-router,api
  assert_in_order router-bootstrap \
    "graphql create-instance scope-repo-router" \
    "domain $scope_args --service scope-repo-router --port 8080 --json" \
    "variable set $scope_args --service scope-api --skip-deploys SCOPE_GIT_PUBLIC_URL=https://scope-repo-router-production.test" \
    "variable set $scope_args --service scope-repo-router --skip-deploys SCOPE_REPO_ROUTER_BACKEND=scope-api.railway.internal:8080 SCOPE_REPO_ROUTER_READ_REPLICAS=1" \
    "graphql configure-scale scope-repo-router" \
    "$m plan" "$up/git-router" "$up/api"
}

router_instance_refused() {
  FAKE_INITIAL_EXACT=1 SCOPE_DEPLOY_WORKER=0 FAKE_ROUTER_CONFIGURED=0 FAKE_ROUTER_INSTANCE_EXISTS=0 \
    run_cutover fail router-instance-refused
  refute router-instance-refused "$no_router_mutation" \
    "an absent git-router instance without a selected git-router must fail before mutation"
}

router_drift_refused() {
  FAKE_INITIAL_EXACT=1 SCOPE_DEPLOY_WORKER=0 FAKE_ROUTER_CONFIGURED=0 run_cutover fail router-drift-refused
  refute router-drift-refused "variable set|up |maintenance apply" "git-router drift without a selected git-router must fail before mutation"
}

router_domain_invalid() {
  FAKE_INITIAL_EXACT=1 SCOPE_DEPLOY_WORKER=0 SCOPE_DEPLOY_ROUTER=1 FAKE_ROUTER_DOMAIN_STATE=invalid \
    run_cutover fail router-domain-invalid
  refute router-domain-invalid "$no_router_mutation" "an invalid git-router domain must fail before mutation"
}

maintenance_forces_all() {
  SCOPE_DEPLOY_WORKER=0 run_cutover pass maintenance-forces-all
  assert_in_order maintenance-forces-all "$m apply" "$up/media-api" "$up/run-worker" "$up/api"
}

transient_plan() {
  FAKE_INITIAL_EXACT=1 FAKE_FAIL_FIRST_PLAN=1 run_cutover pass transient-plan
  assert_count transient-plan 2 "$m plan" -x
}

interrupted() {
  FAKE_FAIL_UP_SERVICE=scope-worker run_cutover fail interrupted
  FAKE_RECOVER_CLOSED_CUTOVER=1 SCOPE_SUCCESSFUL_DEPLOYMENTS="{$(receipt media-api new-scope-media),$router_receipt}" \
    run_cutover pass interrupted
  assert_evidence_components interrupted run-worker,media-api,media-worker,api,git-router,web
  assert_in_order interrupted "$m plan" "$m verify" "$up/media-api" "$up/run-worker" "$up/api"
  refute interrupted "$m apply" "post-commit recovery must not reapply migrations"
}

intentionally_closed() {
  FAKE_INITIAL_EXACT=1 FAKE_INITIAL_CLOSED=1 run_cutover fail intentionally-closed
  refute intentionally-closed "$up/api" "an ordinary deployment must not reopen intentionally closed writers"
}

bootstrap() {
  FAKE_INITIAL_EXACT=1 FAKE_INITIAL_CLOSED=1 FAKE_NO_HISTORY=1 run_cutover pass bootstrap
  assert_in_order bootstrap "$m plan" "$m verify" "$up/media-api" "$up/run-worker" "$up/api"
}

partial_reopen() {
  FAKE_FAIL_UP_SERVICE=scope-api run_cutover fail partial-reopen
  assert_evidence_components partial-reopen ""
  assert_in_order partial-reopen "graphql stop scope-api old-scope-api" "graphql stop scope-worker old-scope-worker" \
    "$up/media-api" "$up/run-worker" "$up/api" "graphql stop scope-media new-scope-media" "graphql stop scope-worker new-scope-worker"
  assert_count partial-reopen 1 "graphql stop scope-api old-scope-api"
  assert_count partial-reopen 2 "gate reclose scope-api"
}

lost_activation_response() {
  FAKE_LOST_UP_RESPONSE_SERVICE=scope-media run_cutover fail lost-activation-response
  assert_evidence_components lost-activation-response ""
  [[ -f "$test_dir/lost-activation-response-state/up-scope-media" ]]
  [[ -f "$test_dir/lost-activation-response-state/stopped-scope-media" ]]
  assert_in_order lost-activation-response "$up/media-api" "graphql stop scope-media new-scope-media"
  refute lost-activation-response "$up/run-worker|$up/api|graphql restart " "failed media deployment must leave all metadata writers closed"
  FAKE_RECOVER_CLOSED_CUTOVER=1 run_cutover pass lost-activation-response
  assert_evidence_components lost-activation-response run-worker,media-api,media-worker,api,git-router,web
}

denied_api() {
  FAKE_DENY_DEPLOYMENT_ACTION_SERVICE=scope-api run_cutover fail denied-api
  assert_in_order denied-api "graphql stop scope-api old-scope-api" "graphql stop scope-worker old-scope-worker"
  refute denied-api 'graphql stop .* gate-' "failure cleanup must preserve serving gate deployments"
  refute denied-api "$m apply" "a denied API shutdown must fail before migration without attempting rollback mutations"
}

denied_worker() {
  FAKE_DENY_DEPLOYMENT_ACTION_SERVICE=scope-worker run_cutover fail denied-worker
  assert_in_order denied-worker "graphql stop scope-api old-scope-api" "graphql stop scope-worker old-scope-worker"
  refute denied-worker "graphql restart scope-worker" "a run-worker shutdown denial must not restore a run-worker that was never closed"
}

denied_media_worker() {
  FAKE_DENY_DEPLOYMENT_ACTION_SERVICE=scope-media-worker run_cutover fail denied-media-worker
  assert_in_order denied-media-worker "graphql stop scope-api old-scope-api" "graphql stop scope-worker old-scope-worker" \
    "graphql stop scope-media-worker old-scope-media-worker"
  refute denied-media-worker "graphql restart scope-media-worker" \
    "a media-worker shutdown denial must not restore a media-worker deployment that was never closed"
}

killed_after_commit() {
  FAKE_KILL_CUTOVER_PHASE=apply run_cutover fail killed-after-commit
  [[ "$(jq -r '.statuses["1"][0].description' "$test_dir/killed-after-commit-state/journal.json")" == "cutover:applying" ]]
  run_cutover fail killed-after-commit
  refute killed-after-commit 'graphql (stop|restart)|up |maintenance' "ordinary deployment must stop at the durable unresolved-cutover guard"
  FAKE_RECOVER_CLOSED_CUTOVER=1 run_cutover pass killed-after-commit
  assert_in_order killed-after-commit "gate reclose scope-api" "graphql stop scope-worker" "$m plan" "$m verify" \
    "$up/media-api" "$up/run-worker" "$up/api"
  refute killed-after-commit "$m apply" "recovery after committed transaction must not reapply migrations"
}

scenarios=(
  disabled_maintenance missing_prepared_worker schema_drift changed_plan
  history_failure_media history_failure_bootstrap history_failure_stopped_media
  success parallel_activation delayed_teardown migration_router_selected stale_stop_status
  draining_writer invalid_workflow_catalog degraded degraded_media wrong_api_region
  degraded_during_shutdown crashed_worker evidence_promotion rollback unknown
  rolling rolling_worker carried_worker_drift rolling_worker_unhealthy
  router_bootstrap router_instance_refused router_drift_refused router_domain_invalid
  maintenance_forces_all transient_plan interrupted intentionally_closed bootstrap
  partial_reopen lost_activation_response denied_api denied_worker denied_media_worker
  killed_after_commit
)

declare -A running=()
failed=()

collect_scenario() {
  local pid status
  if wait -n -p pid; then status=0; else status=$?; fi
  if [[ "$status" != 0 ]]; then
    failed+=("${running[$pid]}")
    printf '%s failed with exit %s:\n' "${running[$pid]}" "$status" >&2
    cat "$test_dir/${running[$pid]}.log" >&2
  fi
  unset "running[$pid]"
}

jobs_limit="${SCOPE_CUTOVER_TEST_JOBS:-$(nproc)}"
for scenario in "${scenarios[@]}"; do
  while (( ${#running[@]} >= jobs_limit )); do collect_scenario; done
  (trap - EXIT; "$scenario") > "$test_dir/$scenario.log" 2>&1 &
  running[$!]="$scenario"
done
while (( ${#running[@]} > 0 )); do collect_scenario; done

if (( ${#failed[@]} > 0 )); then
  echo "backend deployment cutover scenarios failed: ${failed[*]}" >&2
  exit 1
fi
echo "backend deployment cutover tests passed (${#scenarios[@]} scenarios)"
