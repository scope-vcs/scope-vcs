#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT
mkdir -p "$test_dir/bin" "$test_dir/api" "$test_dir/worker" "$test_dir/cache" "$test_dir/router" "$test_dir/media"
printf '%s\n' aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa > "$test_dir/api/.scope-deployment-sha"
printf '%s\n' aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa > "$test_dir/worker/.scope-deployment-sha"
printf '%s\n' aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa > "$test_dir/cache/.scope-deployment-sha"
printf '%s\n' aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa > "$test_dir/router/.scope-deployment-sha"
printf '%s\n' aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa > "$test_dir/media/.scope-deployment-sha"

cat > "$test_dir/services.json" <<'JSON'
{"services":{"api":{"id":"scope-api"},"worker":{"id":"scope-worker"},"cache":{"id":"scope-cache-service"},"router":{"id":"scope-repo-router"},"media":{"id":"scope-media"},"mediaWorker":{"id":"scope-media-worker"}}}
JSON

# Persist the real journal API requests in fake remote storage across runner invocations.
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
    id:123, path:".github/workflows/scope-production-deploy.yml", event:"schedule",
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
printf '%s\n' "$0 $*" >> "$FAKE_RAILWAY_TRACE"
[[ "${DATABASE_URL:-}" == "postgres://public-database.test/scope" ]]

case "${1:-}" in
  plan)
    if [[ "${FAKE_FAIL_FIRST_PLAN:-0}" == "1" && ! -f "$FAKE_RAILWAY_STATE/first-plan-failed" ]]; then
      touch "$FAKE_RAILWAY_STATE/first-plan-failed"
      exit 1
    fi
    if [[ "${FAKE_FAIL_RECOVERY_PLAN:-0}" == "1" && -f "$FAKE_RAILWAY_STATE/apply-attempted" ]]; then
      exit 1
    fi
    if [[ "${FAKE_CHANGE_PLAN:-0}" == "1" && -f "$FAKE_RAILWAY_STATE/plan-read" ]]; then
      echo '{"exact":false,"pending":[{"name":"changed","impact":"maintenance-required"}]}'
      exit 0
    fi
    touch "$FAKE_RAILWAY_STATE/plan-read"
    if [[ -f "$FAKE_RAILWAY_STATE/exact" ]]; then
      echo '{"exact":true,"pending":[]}'
    else
      echo '{"exact":false,"pending":[{"name":"m0033_git_segment_streaming_v2","impact":"maintenance-required"}]}'
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
      [[ -f "$FAKE_RAILWAY_STATE/stop-requested-scope-cache-service" ]]
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
  backfill-git-segments-v2)
    [[ ! -f "$FAKE_RAILWAY_STATE/apply-attempted" ]]
    touch "$FAKE_RAILWAY_STATE/git-segments-v2-backfilled"
    echo '{"gitSegmentsBackfilled":1}'
    ;;
  cleanup-git-segments-v1)
    [[ -f "$FAKE_RAILWAY_STATE/exact" ]]
    echo '{"legacyGitSegmentObjectsDeleted":1}'
    ;;
  backfill-landing-files)
    [[ -f "$FAKE_RAILWAY_STATE/exact" ]]
    touch "$FAKE_RAILWAY_STATE/landing-files-backfilled"
    echo '{"landingFilesBackfilled":1}'
    ;;
  backfill-workflow-catalogs)
    [[ -f "$FAKE_RAILWAY_STATE/exact" ]]
    touch "$FAKE_RAILWAY_STATE/workflow-catalogs-backfilled"
    echo '{"workflowCatalogsBackfilled":1}'
    ;;
  *)
    exit 2
    ;;
esac
FAKE
chmod +x "$test_dir/maintenance"

source "$root/.github/scripts/backend-cutover-test-provider.sh"

run_cutover() {
  local name="$1" fail_apply="$2" initial_exact="${3:-0}"
  local fail_up_service="${4:-}" fail_recovery_plan="${5:-0}"
  local recover_closed_cutover="${6:-0}" initial_closed="${7:-0}" no_history="${8:-0}"
  local _unused_fail_redeploy_service="${9:-}" fail_first_plan="${10:-0}"
  local deny_deployment_action_service="${11:-}" crash_up_service="${12:-}"
  local new_replicas="${13:-1}" degraded_service="${14:-}"
  local degrade_worker_after_api_stop="${15:-0}"
  local reported_api_region="${16:-us-east4-eqdc4a}" stored_api_region="${17:-us-east4-eqdc4a}"
  local deploy_cache="${18:-1}" deploy_worker="${19:-1}" deploy_api="${20:-1}"
  local stuck_fence="${21:-0}" stale_stop_status="${22:-0}"
  local successful_deployments="${23:-}" fail_workflow_validation="${24:-0}"
  local deploy_router="${25:-0}" router_configured="${26:-1}"
  local router_domain_state="${27:-valid}" router_instance_exists="${28:-1}"
  local unhealthy_after_up_service="${29:-}" skip_up_service="${30:-}"
  if [[ -z "$successful_deployments" ]]; then
    successful_deployments='{"api":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"old-scope-api"},"worker":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"old-scope-worker"},"cache":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"old-scope-cache-service"},"media":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"old-scope-media"},"mediaWorker":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"old-scope-media-worker","artifactDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"router":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"old-scope-repo-router"}}'
  fi
  if [[ "$no_history" == "1" ]]; then
    successful_deployments='{"router":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"old-scope-repo-router"}}'
  fi
  local state="$test_dir/$name-state"
  local trace="$test_dir/$name-trace"
  mkdir -p "$state"
  [[ "$initial_exact" == "1" ]] && touch "$state/exact"
  if [[ "$initial_closed" == "1" ]]; then
    touch "$state/stopped-scope-api" "$state/stopped-scope-worker" \
      "$state/stopped-scope-cache-service" "$state/stopped-scope-media" \
      "$state/stopped-scope-media-worker"
  fi
  if [[ "$no_history" == "1" ]]; then
    touch "$state/no-history-scope-api" "$state/no-history-scope-worker" \
      "$state/no-history-scope-cache-service" "$state/no-history-scope-media" \
      "$state/no-history-scope-media-worker"
  fi
  [[ "$router_configured" == "0" ]] && touch "$state/no-history-scope-repo-router"
  : > "$trace"
  local recovery_id=""
  [[ "$recover_closed_cutover" != "1" ]] || recovery_id=1
  node -e '
const fs = require("node:fs");
const sourceSha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const names = { api:"scope-api",worker:"scope-worker",cache:"scope-cache-service",router:"scope-repo-router",media:"scope-media",mediaWorker:"scope-media-worker" };
const components = Object.fromEntries(Object.entries(names).map(([component,serviceId]) => [component,{
serviceId,sourceSha,image:component === "mediaWorker"
  ? `ghcr.io/scope-vcs/scope-media-worker@sha256:${"a".repeat(64)}`
  : `ghcr.io/test/repo/railway-private-${component}@sha256:${"b".repeat(64)}`}]))
const maintenanceSha256 = require("node:crypto").createHash("sha256").update(fs.readFileSync(process.argv[2])).digest("hex");
fs.writeFileSync(process.argv[1], JSON.stringify({schemaVersion:1,sourceSha,components,maintenanceSha256,preparationRunId:"123"}));
' "$test_dir/$name-prepared.json" "$test_dir/maintenance"
  if [[ -n "${FAKE_MISSING_PREPARED_COMPONENT:-}" ]]; then
    jq --arg component "$FAKE_MISSING_PREPARED_COMPONENT" 'del(.components[$component])' \
      "$test_dir/$name-prepared.json" > "$test_dir/$name-prepared.tmp"
    mv "$test_dir/$name-prepared.tmp" "$test_dir/$name-prepared.json"
  fi
  set +e
  PATH="$test_dir/bin:$PATH" \
    NODE_OPTIONS="--require=$test_dir/github-fetch.cjs" \
    GITHUB_TOKEN="test-token" \
    GITHUB_REPOSITORY="test/repo" \
    SCOPE_PREPARED_RELEASE_PATH="$test_dir/$name-prepared.json" \
    SCOPE_RELEASE_CUTOVER_ID="$recovery_id" \
    FAKE_CHANGE_PLAN="${FAKE_CHANGE_PLAN:-0}" \
    FAKE_KILL_CUTOVER_PHASE="${FAKE_KILL_CUTOVER_PHASE:-}" \
    FAKE_RAILWAY_STATE="$state" \
    FAKE_UPLOAD_ROOT="$test_dir" \
    SCOPE_DEPLOYMENT_MANIFEST="$test_dir/services.json" \
    FAKE_RAILWAY_TRACE="$trace" \
    FAKE_FAIL_APPLY="$fail_apply" \
    FAKE_FAIL_UP_SERVICE="$fail_up_service" \
    FAKE_CRASH_UP_SERVICE="$crash_up_service" \
    FAKE_NEW_REPLICAS="$new_replicas" \
    FAKE_DEGRADED_SERVICE="$degraded_service" \
    FAKE_UNHEALTHY_AFTER_UP_SERVICE="$unhealthy_after_up_service" \
    FAKE_SKIP_UP_SERVICE="$skip_up_service" \
    FAKE_DEGRADE_WORKER_AFTER_API_STOP="$degrade_worker_after_api_stop" \
    FAKE_API_REGION="$reported_api_region" \
    FAKE_STORED_API_REGION="$stored_api_region" \
    FAKE_FAIL_RECOVERY_PLAN="$fail_recovery_plan" \
    FAKE_FAIL_FIRST_PLAN="$fail_first_plan" \
    FAKE_STUCK_FENCE="$stuck_fence" \
    FAKE_STALE_STOP_STATUS="$stale_stop_status" \
    FAKE_FAIL_WORKFLOW_VALIDATION="$fail_workflow_validation" \
    FAKE_ROUTER_CONFIGURED="$router_configured" \
    FAKE_ROUTER_DOMAIN_STATE="$router_domain_state" \
    FAKE_ROUTER_INSTANCE_EXISTS="$router_instance_exists" \
    FAKE_DENY_DEPLOYMENT_ACTION_SERVICE="$deny_deployment_action_service" \
    RAILWAY_PROJECT_ID="project-test" \
    RAILWAY_API_TOKEN="token-graphql" \
    RAILWAY_TOKEN="token-project" \
    SCOPE_RAILWAY_ENVIRONMENT_ID="production" \
    SCOPE_RAILWAY_API_SERVICE_ID="scope-api" \
    SCOPE_RAILWAY_WORKER_SERVICE_ID="scope-worker" \
    SCOPE_RAILWAY_CACHE_SERVICE_ID="scope-cache-service" \
    SCOPE_RAILWAY_ROUTER_SERVICE_ID="scope-repo-router" \
    SCOPE_RAILWAY_MEDIA_SERVICE_ID="scope-media" \
    SCOPE_RAILWAY_MEDIA_WORKER_SERVICE_ID="scope-media-worker" \
    SCOPE_RAILWAY_ROUTER_GROUP_ID="runtime-group" \
    SCOPE_RAILWAY_DATABASE_SERVICE_ID="scope-postgres" \
    SCOPE_RAILWAY_API_REGION_ID="us-east4-eqdc4a" \
    SCOPE_RAILWAY_WORKER_REGION_ID="us-east4-eqdc4a" \
    SCOPE_RAILWAY_MEDIA_REGION_ID="us-east4-eqdc4a" \
    SCOPE_MEDIA_WORKER_IMAGE="ghcr.io/scope-vcs/scope-media-worker@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
    SCOPE_DEPLOY_CACHE="$deploy_cache" \
    SCOPE_DEPLOY_WORKER="$deploy_worker" \
    SCOPE_DEPLOY_ROUTER="$deploy_router" \
    SCOPE_DEPLOY_API="$deploy_api" \
    SCOPE_DEPLOY_MEDIA=0 \
    SCOPE_DEPLOY_MEDIA_WORKER=0 \
    SCOPE_SUCCESSFUL_DEPLOYMENTS="$successful_deployments" \
    SCOPE_DEPLOYMENT_EVIDENCE_PATH="$test_dir/$name-evidence.jsonl" \
    SCOPE_MAINTENANCE_OUTAGE_BUDGET_MS="${FAKE_OUTAGE_BUDGET_MS:-180000}" \
    SCOPE_SERVICE_HEALTH_TIMEOUT_SECONDS=0 \
    SCOPE_SERVICE_HEALTH_POLL_SECONDS=0 \
    GITHUB_SHA="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
    SCOPE_WRITER_FENCE_GRACE_SECONDS="0" \
    SCOPE_MAINTENANCE_BINARY="$test_dir/maintenance" \
    bash -c 'export FAKE_CUTOVER_PID=$$; exec bash "$@"' _ \
      "$root/.github/scripts/deploy-backend-railway.sh" \
      "$test_dir/api" "$test_dir/worker" "$test_dir/cache" "$test_dir/router" "$test_dir/media"
  result=$?
  set -e
  printf '%s\n' "$result" > "$test_dir/$name-result"
}

assert_evidence_components() {
  local name="$1"
  local expected="$2"
  EVIDENCE_PATH="$test_dir/$name-evidence.jsonl" EXPECTED_COMPONENTS="$expected" node -e '
const { existsSync, readFileSync } = require("node:fs");
const actual = existsSync(process.env.EVIDENCE_PATH)
  ? readFileSync(process.env.EVIDENCE_PATH, "utf8").trim().split("\n").filter(Boolean)
      .map((line) => JSON.parse(line).component)
  : [];
const expected = (process.env.EXPECTED_COMPONENTS || "").split(",").filter(Boolean);
if (JSON.stringify(actual) !== JSON.stringify(expected)) {
  console.error(`expected evidence ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  process.exit(1);
}
'
}

assert_in_order() {
  local trace="$1"
  shift
  local previous=0 pattern line
  for pattern in "$@"; do
    line="$(
      grep -n -F "$pattern" "$trace" \
        | cut -d: -f1 \
        | awk -v previous="$previous" '$1 > previous { print; exit }'
    )"
    [[ -n "$line" && "$line" -gt "$previous" ]] || {
      echo "missing or out-of-order '$pattern' in $trace" >&2
      return 1
    }
    previous="$line"
  done
}

FAKE_OUTAGE_BUDGET_MS=0 run_cutover unapproved-budget 0
[[ "$(cat "$test_dir/unapproved-budget-result")" != "0" ]]
if grep -E 'graphql stop|maintenance apply' "$test_dir/unapproved-budget-trace"; then
  echo "unapproved outage budget must prevent closure" >&2
  exit 1
fi

FAKE_MISSING_PREPARED_COMPONENT=cache run_cutover missing-prepared-cache 0
[[ "$(cat "$test_dir/missing-prepared-cache-result")" != "0" ]]
if grep -E 'graphql stop|maintenance apply' "$test_dir/missing-prepared-cache-trace"; then
  echo "missing prepared artifacts must prevent closure" >&2
  exit 1
fi
FAKE_CHANGE_PLAN=1 run_cutover changed-plan 0
[[ "$(cat "$test_dir/changed-plan-result")" != "0" ]]
if grep -E 'graphql stop|maintenance apply' "$test_dir/changed-plan-trace"; then
  echo "changed preclosure plan must prevent closure" >&2
  exit 1
fi

run_cutover success 0
[[ "$(cat "$test_dir/success-result")" == "0" ]]
assert_evidence_components success cache,worker,media,mediaWorker,api
if grep -F -- '--path-as-root' "$test_dir/success-trace"; then
  echo "maintenance activation must not upload a build context" >&2
  exit 1
fi
assert_in_order "$test_dir/success-trace" \
  "$test_dir/maintenance plan" \
  "graphql stop scope-api old-scope-api" \
  "graphql stop scope-worker old-scope-worker" \
  "graphql stop scope-cache-service old-scope-cache-service" \
  "$test_dir/maintenance fence" \
  "$test_dir/maintenance validate-workflow-catalogs" \
  "$test_dir/maintenance backfill-git-segments-v2" \
  "$test_dir/maintenance apply" \
  "$test_dir/maintenance verify" \
  "$test_dir/maintenance cleanup-git-segments-v1" \
  "$test_dir/maintenance backfill-landing-files" \
  "$test_dir/maintenance backfill-workflow-catalogs" \
  "up $test_dir/cache" \
  "up $test_dir/worker" \
  "up $test_dir/api"

run_cutover stale-stop-status 0 0 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 1 1 1 0 1
[[ "$(cat "$test_dir/stale-stop-status-result")" == "0" ]]
assert_in_order "$test_dir/stale-stop-status-trace" \
  "graphql stop scope-api old-scope-api" \
  "graphql stop scope-worker old-scope-worker" \
  "graphql stop scope-cache-service old-scope-cache-service" \
  "$test_dir/maintenance fence" \
  "$test_dir/maintenance apply"

run_cutover draining-writer 0 0 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 1 1 1 1
[[ "$(cat "$test_dir/draining-writer-result")" == "0" ]]
[[ "$(grep -F -x -c "$test_dir/maintenance fence" "$test_dir/draining-writer-trace")" == "2" ]]
assert_in_order "$test_dir/draining-writer-trace" \
  "graphql stop scope-worker old-scope-worker" \
  "graphql stop scope-cache-service old-scope-cache-service" \
  "$test_dir/maintenance fence" \
  "$test_dir/maintenance drain-writers" \
  "$test_dir/maintenance fence" \
  "$test_dir/maintenance validate-workflow-catalogs" \
  "$test_dir/maintenance apply"

run_cutover invalid-workflow-catalog 0 0 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 1 1 1 0 0 "" 1
[[ "$(cat "$test_dir/invalid-workflow-catalog-result")" != "0" ]]
assert_in_order "$test_dir/invalid-workflow-catalog-trace" \
  "graphql stop scope-api old-scope-api" \
  "graphql stop scope-worker old-scope-worker" \
  "graphql stop scope-cache-service old-scope-cache-service" \
  "$test_dir/maintenance fence" \
  "$test_dir/maintenance validate-workflow-catalogs" \
  "graphql restart scope-cache-service old-scope-cache-service" \
  "graphql restart scope-worker old-scope-worker" \
  "graphql restart scope-api old-scope-api"
if grep -F "$test_dir/maintenance apply" "$test_dir/invalid-workflow-catalog-trace"; then
  echo "invalid workflow catalogs must fail before migration" >&2
  exit 1
fi

run_cutover degraded 0 0 "" 0 0 0 0 "" 0 "" "" 1 scope-worker
[[ "$(cat "$test_dir/degraded-result")" != "0" ]]
if grep -E "graphql (stop|restart) " "$test_dir/degraded-trace" \
  || grep -F "$test_dir/maintenance apply" "$test_dir/degraded-trace"; then
  echo "maintenance must not start from a degraded service" >&2
  exit 1
fi

run_cutover degraded-cache 0 0 "" 0 0 0 0 "" 0 "" "" 1 scope-cache-service
[[ "$(cat "$test_dir/degraded-cache-result")" != "0" ]]
if grep -E "graphql (stop|restart) " "$test_dir/degraded-cache-trace" \
  || grep -F "$test_dir/maintenance apply" "$test_dir/degraded-cache-trace"; then
  echo "maintenance must not start with a degraded cache writer" >&2
  exit 1
fi

run_cutover wrong-api-region 0 0 "" 0 0 0 0 "" 0 "" "" 1 "" 0 us-west2
[[ "$(cat "$test_dir/wrong-api-region-result")" != "0" ]]
if grep -E "graphql (stop|restart) " "$test_dir/wrong-api-region-trace" \
  || grep -F "$test_dir/maintenance apply" "$test_dir/wrong-api-region-trace"; then
  echo "region drift must fail before maintenance starts" >&2
  exit 1
fi

run_cutover wrong-stored-api-region 0 0 "" 0 0 0 0 "" 0 "" "" 1 "" 0 us-east4-eqdc4a us-west2
[[ "$(cat "$test_dir/wrong-stored-api-region-result")" != "0" ]]
if grep -E "graphql (stop|restart) " "$test_dir/wrong-stored-api-region-trace" \
  || grep -F "$test_dir/maintenance apply" "$test_dir/wrong-stored-api-region-trace"; then
  echo "stored region drift must fail before maintenance starts" >&2
  exit 1
fi

run_cutover degraded-during-shutdown 0 0 "" 0 0 0 0 "" 0 "" "" 1 "" 1
[[ "$(cat "$test_dir/degraded-during-shutdown-result")" != "0" ]]
assert_in_order "$test_dir/degraded-during-shutdown-trace" \
  "graphql stop scope-api old-scope-api" \
  "graphql restart scope-api old-scope-api"
if grep -F "graphql stop scope-worker" "$test_dir/degraded-during-shutdown-trace"; then
  echo "a service that degrades before shutdown must remain recoverable" >&2
  exit 1
fi

run_cutover crashed-worker 0 0 "" 0 0 0 0 "" 0 "" scope-worker
[[ "$(cat "$test_dir/crashed-worker-result")" != "0" ]]
assert_evidence_components crashed-worker cache
[[ -f "$test_dir/crashed-worker-state/stopped-scope-worker" ]]
assert_in_order "$test_dir/crashed-worker-trace" \
  "$test_dir/maintenance apply" \
  "up $test_dir/worker" \
  "graphql stop scope-cache-service new-scope-cache-service"

run_cutover crashed-cache 0 0 "" 0 0 0 0 "" 0 "" scope-cache-service
[[ "$(cat "$test_dir/crashed-cache-result")" != "0" ]]
assert_evidence_components crashed-cache ""
[[ -f "$test_dir/crashed-cache-state/stopped-scope-cache-service" ]]
assert_in_order "$test_dir/crashed-cache-trace" \
  "$test_dir/maintenance apply" \
  "up $test_dir/cache"
if grep -F "up $test_dir/worker" "$test_dir/crashed-cache-trace" \
  || grep -F "up $test_dir/api" "$test_dir/crashed-cache-trace" \
  || grep -F "graphql restart " "$test_dir/crashed-cache-trace"; then
  echo "failed cache deployment must leave all metadata writers closed" >&2
  exit 1
fi

mkdir "$test_dir/cache-evidence-promotion-evidence.jsonl"
run_cutover cache-evidence-promotion 0 2> "$test_dir/cache-evidence-promotion-stderr"
[[ "$(cat "$test_dir/cache-evidence-promotion-result")" != "0" ]]
[[ -f "$test_dir/cache-evidence-promotion-state/up-scope-cache-service" ]]
[[ -f "$test_dir/cache-evidence-promotion-state/stopped-scope-cache-service" ]]
assert_in_order "$test_dir/cache-evidence-promotion-trace" \
  "$test_dir/maintenance apply" \
  "up $test_dir/cache" \
  "graphql stop scope-cache-service new-scope-cache-service" \
  "$test_dir/maintenance fence"
[[ "$(grep -F -c "graphql stop scope-cache-service new-scope-cache-service" \
  "$test_dir/cache-evidence-promotion-trace")" == "1" ]]
if grep -E "up $test_dir/(worker|api)" "$test_dir/cache-evidence-promotion-trace"; then
  echo "failed cache evidence promotion must stop before worker or API activation" >&2
  exit 1
fi
if compgen -G "$test_dir/cache-evidence-promotion-evidence.jsonl.pending.*" > /dev/null; then
  echo "failed cache evidence promotion must discard pending evidence" >&2
  exit 1
fi
[[ -z "$(find "$test_dir/cache-evidence-promotion-evidence.jsonl" -mindepth 1 -print -quit)" ]]
grep -F \
  "Migration committed; writers remain closed. Rerun this workflow to finish the forward-only deployment." \
  "$test_dir/cache-evidence-promotion-stderr"
if grep -F "Failed to re-close metadata writers after the committed cutover." \
  "$test_dir/cache-evidence-promotion-stderr"; then
  echo "cache evidence promotion recovery must report successful writer closure" >&2
  exit 1
fi

run_cutover rollback rollback
[[ "$(cat "$test_dir/rollback-result")" != "0" ]]
assert_in_order "$test_dir/rollback-trace" \
  "$test_dir/maintenance apply" \
  "$test_dir/maintenance plan" \
  "graphql restart scope-cache-service old-scope-cache-service" \
  "graphql restart scope-worker old-scope-worker" \
  "graphql restart scope-api old-scope-api"
if grep -F "up $test_dir/api" "$test_dir/rollback-trace"; then
  echo "failed migration must not deploy the new API" >&2
  exit 1
fi

run_cutover unknown committed-error 0 "" 1
[[ "$(cat "$test_dir/unknown-result")" != "0" ]]
if grep -F "graphql restart " "$test_dir/unknown-trace"; then
  echo "unknown migration state must not restore old deployments" >&2
  exit 1
fi
if grep -F "up $test_dir/api" "$test_dir/unknown-trace"; then
  echo "unknown migration state must not deploy new binaries" >&2
  exit 1
fi

run_cutover rolling 0 1
[[ "$(cat "$test_dir/rolling-result")" == "0" ]]
assert_in_order "$test_dir/rolling-trace" \
  "$test_dir/maintenance plan" \
  "$test_dir/maintenance verify" \
  "up $test_dir/cache" \
  "up $test_dir/worker" \
  "up $test_dir/api"
if grep -E "graphql (stop|restart) " "$test_dir/rolling-trace"; then
  echo "exact-schema deployment must stay on the rolling path" >&2
  exit 1
fi

run_cutover rolling-cache 0 1 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 1 0 0
[[ "$(cat "$test_dir/rolling-cache-result")" == "0" ]]
assert_in_order "$test_dir/rolling-cache-trace" \
  "$test_dir/maintenance verify" \
  "up $test_dir/cache"
if grep -E "up $test_dir/(worker|api)" "$test_dir/rolling-cache-trace"; then
  echo "cache-only deployment must not deploy worker or API" >&2
  exit 1
fi

run_cutover rolling-worker 0 1 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 1 0
[[ "$(cat "$test_dir/rolling-worker-result")" == "0" ]]
assert_evidence_components rolling-worker worker
assert_in_order "$test_dir/rolling-worker-trace" \
  "$test_dir/maintenance verify" \
  "up $test_dir/worker"
if grep -E "up $test_dir/(cache|api)" "$test_dir/rolling-worker-trace"; then
  echo "worker-only deployment must not deploy cache or API" >&2
  exit 1
fi

run_cutover carried-worker-drift 0 1 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 1 0 0 0 \
  '{"worker":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"new-scope-worker"},"router":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"old-scope-repo-router"}}' \
  0 0 1 valid 1 "" scope-worker
[[ "$(cat "$test_dir/carried-worker-drift-result")" != "0" ]]
assert_evidence_components carried-worker-drift ""
if grep -F "graphql restart scope-worker" "$test_dir/carried-worker-drift-trace"; then
  echo "a skipped deploy with drifted active identity must not restart the carried deployment" >&2
  exit 1
fi

run_cutover rolling-worker-unhealthy 0 1 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 1 0 0 0 "" 0 0 1 valid 1 scope-worker
[[ "$(cat "$test_dir/rolling-worker-unhealthy-result")" != "0" ]]
assert_evidence_components rolling-worker-unhealthy ""

run_cutover rolling-api 0 1 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 0 1
[[ "$(cat "$test_dir/rolling-api-result")" == "0" ]]
assert_in_order "$test_dir/rolling-api-trace" \
  "$test_dir/maintenance verify" \
  "up $test_dir/api"
if grep -E "up $test_dir/(cache|worker)" "$test_dir/rolling-api-trace"; then
  echo "API-only deployment must not deploy cache or worker" >&2
  exit 1
fi

run_cutover rolling-api-unhealthy 0 1 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 0 1 0 0 "" 0 0 1 valid 1 scope-api
[[ "$(cat "$test_dir/rolling-api-unhealthy-result")" != "0" ]]
assert_evidence_components rolling-api-unhealthy ""

run_cutover router-bootstrap 0 1 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 0 1 0 0 "" 0 1 0 valid 0
[[ "$(cat "$test_dir/router-bootstrap-result")" == "0" ]]
assert_evidence_components router-bootstrap router,api
assert_in_order "$test_dir/router-bootstrap-trace" \
  "graphql create-instance scope-repo-router" \
  "domain --project project-test --environment production --service scope-repo-router --port 8080 --json" \
  "variable set --project project-test --environment production --service scope-api --skip-deploys SCOPE_GIT_PUBLIC_URL=https://scope-repo-router-production.test" \
  "variable set --project project-test --environment production --service scope-repo-router --skip-deploys SCOPE_REPO_ROUTER_BACKEND=scope-api.railway.internal:8080 SCOPE_REPO_ROUTER_READ_REPLICAS=1" \
  "graphql configure-scale scope-repo-router" \
  "up $test_dir/router" \
  "$test_dir/maintenance plan" \
  "up $test_dir/api"

run_cutover router-instance-refused 0 1 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 0 1 0 0 "" 0 0 0 valid 0
[[ "$(cat "$test_dir/router-instance-refused-result")" != "0" ]]
if grep -E "graphql create-instance|domain --|variable set|up |maintenance" \
  "$test_dir/router-instance-refused-trace"; then
  echo "an absent router instance without a selected router must fail before mutation" >&2
  exit 1
fi

run_cutover router-drift-refused 0 1 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 0 1 0 0 "" 0 0 0
[[ "$(cat "$test_dir/router-drift-refused-result")" != "0" ]]
if grep -E "variable set|up |maintenance" "$test_dir/router-drift-refused-trace"; then
  echo "router drift without a selected router must fail before mutation" >&2
  exit 1
fi

run_cutover router-domain-invalid 0 1 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 0 1 0 0 "" 0 1 1 invalid
[[ "$(cat "$test_dir/router-domain-invalid-result")" != "0" ]]
if grep -E "domain --|variable set|up |maintenance" "$test_dir/router-domain-invalid-trace"; then
  echo "an invalid router domain must fail before mutation" >&2
  exit 1
fi

run_cutover maintenance-forces-all 0 0 "" 0 0 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 0 1
[[ "$(cat "$test_dir/maintenance-forces-all-result")" == "0" ]]
assert_in_order "$test_dir/maintenance-forces-all-trace" \
  "$test_dir/maintenance apply" \
  "up $test_dir/cache" \
  "up $test_dir/worker" \
  "up $test_dir/api"

run_cutover transient-plan 0 1 "" 0 0 0 0 "" 1
[[ "$(cat "$test_dir/transient-plan-result")" == "0" ]]
[[ "$(grep -F -x -c "$test_dir/maintenance plan" "$test_dir/transient-plan-trace")" == "2" ]]

run_cutover interrupted 0 0 scope-worker
[[ "$(cat "$test_dir/interrupted-result")" != "0" ]]
run_cutover interrupted 0 0 "" 0 1 0 0 "" 0 "" "" 1 "" 0 \
  us-east4-eqdc4a us-east4-eqdc4a 0 1 1 0 0 \
  '{"cache":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"new-scope-cache-service"},"router":{"sourceSha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","provider":"railway","evidenceId":"old-scope-repo-router"}}'
[[ "$(cat "$test_dir/interrupted-result")" == "0" ]]
assert_evidence_components interrupted cache,cache,worker,media,mediaWorker,api
assert_in_order "$test_dir/interrupted-trace" \
  "$test_dir/maintenance plan" \
  "$test_dir/maintenance verify" \
  "up $test_dir/cache" \
  "up $test_dir/worker" \
  "up $test_dir/api"
if grep -F "$test_dir/maintenance apply" "$test_dir/interrupted-trace"; then
  echo "post-commit recovery must not reapply migrations" >&2
  exit 1
fi

run_cutover intentionally-closed 0 1 "" 0 0 1
[[ "$(cat "$test_dir/intentionally-closed-result")" != "0" ]]
if grep -F "up $test_dir/api" "$test_dir/intentionally-closed-trace"; then
  echo "an ordinary deployment must not reopen intentionally closed writers" >&2
  exit 1
fi

run_cutover bootstrap 0 1 "" 0 0 1 1
[[ "$(cat "$test_dir/bootstrap-result")" == "0" ]]
assert_in_order "$test_dir/bootstrap-trace" \
  "$test_dir/maintenance plan" \
  "$test_dir/maintenance verify" \
  "up $test_dir/cache" \
  "up $test_dir/worker" \
  "up $test_dir/api"

run_cutover partial-reopen 0 0 scope-api
[[ "$(cat "$test_dir/partial-reopen-result")" != "0" ]]
assert_evidence_components partial-reopen cache
assert_in_order "$test_dir/partial-reopen-trace" \
  "graphql stop scope-api old-scope-api" \
  "graphql stop scope-worker old-scope-worker" \
  "up $test_dir/cache" \
  "up $test_dir/worker" \
  "up $test_dir/api" \
  "graphql stop scope-worker new-scope-worker" \
  "graphql stop scope-cache-service new-scope-cache-service"
[[ "$(grep -F -c "graphql stop scope-api old-scope-api" "$test_dir/partial-reopen-trace")" == "2" ]]

run_cutover denied-api 0 0 "" 0 0 0 0 "" 0 scope-api
[[ "$(cat "$test_dir/denied-api-result")" != "0" ]]
[[ "$(grep -F -c "graphql stop " "$test_dir/denied-api-trace")" == "1" ]]
if grep -F "$test_dir/maintenance apply" "$test_dir/denied-api-trace"; then
  echo "a denied API shutdown must fail before migration without attempting rollback mutations" >&2
  exit 1
fi

run_cutover denied-worker 0 0 "" 0 0 0 0 "" 0 scope-worker
[[ "$(cat "$test_dir/denied-worker-result")" != "0" ]]
assert_in_order "$test_dir/denied-worker-trace" \
  "graphql stop scope-api old-scope-api" \
  "graphql stop scope-worker old-scope-worker" \
  "graphql restart scope-api old-scope-api"
if grep -F "graphql restart scope-worker" "$test_dir/denied-worker-trace"; then
  echo "a worker shutdown denial must not restore a worker that was never closed" >&2
  exit 1
fi

run_cutover denied-cache 0 0 "" 0 0 0 0 "" 0 scope-cache-service
[[ "$(cat "$test_dir/denied-cache-result")" != "0" ]]
assert_in_order "$test_dir/denied-cache-trace" \
  "graphql stop scope-api old-scope-api" \
  "graphql stop scope-worker old-scope-worker" \
  "graphql stop scope-cache-service old-scope-cache-service" \
  "graphql restart scope-worker old-scope-worker" \
  "graphql restart scope-api old-scope-api"
if grep -F "graphql restart scope-cache-service" "$test_dir/denied-cache-trace"; then
  echo "a cache shutdown denial must not restore a cache deployment that was never closed" >&2
  exit 1
fi

FAKE_KILL_CUTOVER_PHASE=apply run_cutover killed-after-commit 0
[[ "$(cat "$test_dir/killed-after-commit-result")" != "0" ]]
[[ "$(jq -r '.statuses["1"][0].description' "$test_dir/killed-after-commit-state/journal.json")" == "cutover:applying" ]]
run_cutover killed-after-commit 0
[[ "$(cat "$test_dir/killed-after-commit-result")" != "0" ]]
if grep -E 'graphql (stop|restart)|up |maintenance' "$test_dir/killed-after-commit-trace"; then
  echo "ordinary deployment must stop at the durable unresolved-cutover guard" >&2
  exit 1
fi
FAKE_OUTAGE_BUDGET_MS=0 run_cutover killed-after-commit 0 0 "" 0 1
[[ "$(cat "$test_dir/killed-after-commit-result")" == "0" ]]
assert_in_order "$test_dir/killed-after-commit-trace" \
  "graphql stop scope-api" \
  "graphql stop scope-worker" \
  "graphql stop scope-cache-service" \
  "$test_dir/maintenance plan" \
  "$test_dir/maintenance verify" \
  "up $test_dir/cache" \
  "up $test_dir/worker" \
  "up $test_dir/api"
if grep -F "$test_dir/maintenance apply" "$test_dir/killed-after-commit-trace"; then
  echo "recovery after committed transaction must not reapply migrations" >&2
  exit 1
fi

direct_state="$test_dir/direct-state"
direct_trace="$test_dir/direct-trace"
direct_evidence="$test_dir/direct-evidence.jsonl"
mkdir -p "$direct_state"
: > "$direct_trace"
run_direct_deploy() {
  PATH="$test_dir/bin:$PATH" \
    FAKE_RAILWAY_STATE="$direct_state" \
    FAKE_RAILWAY_TRACE="$direct_trace" \
    RAILWAY_PROJECT_ID="project-test" \
    RAILWAY_TOKEN="token-project" \
    SCOPE_RAILWAY_ENVIRONMENT_ID="production" \
    SCOPE_DEPLOYMENT_COMPONENT="api" \
    SCOPE_DEPLOYMENT_SOURCE_SHA="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
    SCOPE_VERIFIED_SUCCESSFUL_SHA="${1:-}" \
    SCOPE_DEPLOYMENT_EVIDENCE_PATH="${2:-}" \
    bash "$root/.github/scripts/deploy-railway.sh" scope-api "$test_dir/api"
}

run_direct_deploy "" "$direct_evidence"
EVIDENCE_PATH="$direct_evidence" node -e '
const { readFileSync } = require("node:fs");
const evidence = JSON.parse(readFileSync(process.env.EVIDENCE_PATH, "utf8"));
if (evidence.component !== "api" || evidence.sourceSha !== "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" ||
    evidence.provider !== "railway" || evidence.evidenceId !== "new-scope-api") process.exit(1);
'

# A healthy old service is not proof that Railway deployed the requested source revision.
set +e
run_direct_deploy
skipped_result=$?
set -e
[[ "$skipped_result" != "0" ]]

# Exact durable identity plus current health makes an identical deployment idempotent.
run_direct_deploy aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa "$direct_evidence"
[[ "$(wc -l < "$direct_evidence")" == "1" ]]

# Historical identity must not carry a deployment whose live service has crashed.
touch "$direct_state/crashed-scope-api"
set +e
run_direct_deploy aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
unhealthy_skipped_result=$?
set -e
[[ "$unhealthy_skipped_result" != "0" ]]

echo "backend deployment cutover tests passed"
