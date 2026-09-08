#!/usr/bin/env bash
# Railway target validation, topology, health, and deployment control.

validate_production_target() {
  local status_json services_json environment_config_json
  status_json="$(railway status "${railway_scope[@]}" --json)"
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  environment_config_json="$(railway environment config --environment "$environment" --json)"
  # The JavaScript template literals are evaluated by Node.
  # shellcheck disable=SC2016
  RAILWAY_STATUS_JSON="$status_json" \
    RAILWAY_SERVICES_JSON="$services_json" \
    RAILWAY_ENVIRONMENT_CONFIG_JSON="$environment_config_json" \
    EXPECTED_PROJECT_ID="$RAILWAY_PROJECT_ID" \
    EXPECTED_ENVIRONMENT_ID="$environment" \
    EXPECTED_API_SERVICE_ID="$api_service" \
    EXPECTED_WORKER_SERVICE_ID="$worker_service" \
    EXPECTED_CACHE_SERVICE_ID="$cache_service" \
    EXPECTED_ROUTER_SERVICE_ID="$router_service" \
    EXPECTED_DATABASE_SERVICE_ID="$database_service" \
    EXPECTED_API_REGION="$api_region" \
    EXPECTED_WORKER_REGION="$worker_region" \
    node -e '
const status = JSON.parse(process.env.RAILWAY_STATUS_JSON || "{}");
const serviceStates = JSON.parse(process.env.RAILWAY_SERVICES_JSON || "[]");
const environmentConfig = JSON.parse(process.env.RAILWAY_ENVIRONMENT_CONFIG_JSON || "{}");
const fail = (message) => {
  console.error(`Refusing backend deployment: ${message}.`);
  process.exit(1);
};
const expectedServices = new Map([
  [process.env.EXPECTED_API_SERVICE_ID, "scope-api"],
  [process.env.EXPECTED_WORKER_SERVICE_ID, "scope-worker"],
  [process.env.EXPECTED_CACHE_SERVICE_ID, "scope-cache-service"],
  [process.env.EXPECTED_ROUTER_SERVICE_ID, "scope-repo-router"],
  [process.env.EXPECTED_DATABASE_SERVICE_ID, "scope-postgres"],
]);
const environments = status.environments?.edges?.map(({node}) => node) || [];
const services = status.services?.edges?.map(({node}) => node) || [];
if (status.id !== process.env.EXPECTED_PROJECT_ID) fail("Railway project ID does not match the reviewed target");
if (!environments.some(({id, name}) => id === process.env.EXPECTED_ENVIRONMENT_ID && name === "production")) {
  fail("Railway production environment does not match the reviewed target");
}
for (const [id, name] of expectedServices) {
  if (!services.some((service) => service.id === id && service.name === name)) {
    fail(`Railway service ${name} does not match the reviewed target`);
  }
}
for (const [id, name, expectedRegion] of [
  [process.env.EXPECTED_API_SERVICE_ID, "scope-api", process.env.EXPECTED_API_REGION],
  [process.env.EXPECTED_WORKER_SERVICE_ID, "scope-worker", process.env.EXPECTED_WORKER_REGION],
]) {
  const service = serviceStates.find((candidate) => candidate.id === id);
  if (!service) fail(`Railway service ${name} has no production state`);
  const configured = service.replicas?.configured || 0;
  if (configured > 0 && !service.regions?.some(
    (region) => region.name === expectedRegion && region.configured === configured,
  )) {
    fail(`Railway service ${name} is not running in reviewed region ${expectedRegion}`);
  }
  const storedRegions = environmentConfig.services?.[id]?.deploy?.multiRegionConfig || {};
  const activeStoredRegions = Object.entries(storedRegions).filter(
    ([, config]) => config && Number(config.numReplicas) > 0,
  );
  if (configured > 0 && !(
    activeStoredRegions.length === 1 &&
    activeStoredRegions[0][0] === expectedRegion &&
    Number(activeStoredRegions[0][1].numReplicas) === configured
  )) {
    fail(`Railway service ${name} stored region config does not match reviewed region ${expectedRegion}`);
  }
}
'
}

service_state_line() {
  local service_name="$1"
  local services_json
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  SERVICES_JSON="$services_json" SERVICE_NAME="$service_name" node -e '
const services = JSON.parse(process.env.SERVICES_JSON || "[]");
const target = process.env.SERVICE_NAME;
const service = services.find((item) => item.id === target || item.name === target);
if (!service) process.exit(1);
const replicas = service.replicas || {};
console.log([
  service.status || "",
  replicas.running || 0,
  replicas.crashed || 0,
  replicas.configured || 0,
  service.deploymentStopped === true ? "1" : "0",
  service.deploymentId || "",
].join("\t"));
'
}

wait_for_service_health() {
  local service_name="$1"
  local expected_deployment_id="${2:-}"
  local verify_config="${3:-0}"
  local timeout="${SCOPE_SERVICE_HEALTH_TIMEOUT_SECONDS:-600}"
  local interval="${SCOPE_SERVICE_HEALTH_POLL_SECONDS:-10}"
  local deadline=$((SECONDS + timeout))
  while true; do
    if service_is_healthy "$service_name" "$expected_deployment_id" "$verify_config" 2>/dev/null; then
      return 0
    fi
    (( SECONDS < deadline )) || break
    sleep "$interval"
  done
  service_is_healthy "$service_name" "$expected_deployment_id" "$verify_config" || true
  echo "Timed out waiting for $service_name to reach its configured healthy replica count." >&2
  return 1
}

service_is_healthy() {
  local service_name="$1"
  local expected_deployment_id="${2:-}"
  local services_json
  local verify_config="${3:-0}"
  local expected_config
  case "$service_name" in
    "$api_service") expected_config=api/railway.json ;;
    "$worker_service") expected_config=worker/railway.json ;;
    "$cache_service") expected_config=cache-service/railway.json ;;
    "$router_service") expected_config=repo-router/railway.json ;;
    *) echo "Unknown backend service: $service_name" >&2; return 1 ;;
  esac
  [[ "$verify_config" == "1" ]] || expected_config=""
  services_json="$(railway status "${railway_scope[@]}" --json)"
  SCOPE_RAILWAY_ENVIRONMENT_ID="$environment" \
    SCOPE_EXPECTED_RAILWAY_CONFIG="$expected_config" \
    SCOPE_RAILWAY_SERVICES_JSON="$services_json" \
    SCOPE_RAILWAY_SERVICE_ID="$service_name" \
    SCOPE_EXPECTED_RAILWAY_DEPLOYMENT_ID="$expected_deployment_id" \
    node .github/scripts/railway-service-health.mjs >/dev/null
}

running_replicas() {
  local line status running crashed configured stopped id
  line="$(service_state_line "$1")"
  IFS=$'\t' read -r status running crashed configured stopped id <<< "$line"
  printf '%s\n' "$running"
}

configured_replicas() {
  local line status running crashed configured stopped id
  line="$(service_state_line "$1")"
  IFS=$'\t' read -r status running crashed configured stopped id <<< "$line"
  printf '%s\n' "$configured"
}

router_public_domain() {
  jq -er '
    [.domains[]? | select(.type == "service")]
    | if length != 1 then error("expected exactly one router service domain")
      elif .[0].syncStatus != "ACTIVE" then error("router service domain is not active")
      elif .[0].targetPort != 8080 then error("router service domain does not target port 8080")
      else .[0].domain
      end
  ' <<< "$1"
}

production_service_exists() {
  local service_id="$1"
  local services_json
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  SERVICES_JSON="$services_json" SERVICE_ID="$service_id" node -e '
const services = JSON.parse(process.env.SERVICES_JSON || "[]");
process.exit(services.some(({id}) => id === process.env.SERVICE_ID) ? 0 : 1);
'
}

commit_environment_patch() {
  local patch="$1"
  local commit_message="$2"
  local mutation variables response
  mutation='mutation UpdateProductionEnvironment(
    $environmentId: String!,
    $patch: EnvironmentConfig!,
    $commitMessage: String,
  ) {
    environmentPatchCommit(
      environmentId: $environmentId,
      patch: $patch,
      commitMessage: $commitMessage,
    )
  }'
  variables="$(
    jq -cn \
      --arg environmentId "$environment" \
      --argjson patch "$patch" \
      --arg commitMessage "$commit_message" \
      '{environmentId: $environmentId, patch: $patch, commitMessage: $commitMessage}'
  )"
  response="$(
    env -u RAILWAY_TOKEN \
      RAILWAY_API_TOKEN="$railway_api_token" \
      railway api "$mutation" --variables "$variables" --compact
  )"
  if ! jq -e '.data.environmentPatchCommit | type == "string" and length > 0' \
    <<< "$response" >/dev/null; then
    echo "Railway environment patch failed: $commit_message." >&2
    return 1
  fi
}

ensure_production_router_instance() {
  local patch deadline
  if production_service_exists "$router_service"; then
    return 0
  fi
  if [[ "$deploy_router_requested" != "1" ]]; then
    echo "Production router has no service instance; select the router deployment." >&2
    return 1
  fi

  patch="$(
    jq -cn --arg service "$router_service" --arg group "$router_group" \
      '{services: {($service): {isCreated: true, groupId: $group}}}'
  )"
  commit_environment_patch "$patch" "Create production Git router instance"

  deadline=$((SECONDS + 60))
  while (( SECONDS < deadline )); do
    if production_service_exists "$router_service"; then
      return 0
    fi
    sleep 2
  done
  echo "Production router service instance did not appear after creation." >&2
  return 1
}

router_service_config_matches() {
  local environment_config_json
  environment_config_json="$(railway environment config --environment "$environment" --json)"
  jq -e \
    --arg service "$router_service" \
    --arg group "$router_group" \
    --arg region "$api_region" \
    '.services[$service].groupId == $group and
      ((.services[$service].deploy.multiRegionConfig // {})
        | to_entries
        | map(select((.value.numReplicas // 0 | tonumber) > 0))
        | length == 1 and .[0].key == $region and (.[0].value.numReplicas | tonumber) == 1)' \
    <<< "$environment_config_json" >/dev/null
}

configure_router_service() {
  local patch deadline
  if router_service_config_matches; then
    return 0
  fi
  if [[ "$deploy_router_requested" != "1" ]]; then
    echo "Production router service configuration drift requires a router deployment." >&2
    return 1
  fi
  patch="$(
    jq -cn \
      --arg service "$router_service" \
      --arg group "$router_group" \
      --arg region "$api_region" \
      '{services: {($service): {
        groupId: $group,
        deploy: {multiRegionConfig: {($region): {numReplicas: 1}}}
      }}}'
  )"
  commit_environment_patch "$patch" "Configure production Git router topology"
  deadline=$((SECONDS + 60))
  while (( SECONDS < deadline )); do
    if router_service_config_matches; then
      return 0
    fi
    sleep 2
  done
  echo "Production router service configuration did not converge after update." >&2
  return 1
}

configure_production_router() {
  local domains router_domain router_url api_replicas api_variables router_variables
  domains="$(railway domain list "${railway_scope[@]}" --service "$router_service" --json)"
  if [[ "$(jq '[.domains[]? | select(.type == "service")] | length' <<< "$domains")" == "0" ]]; then
    if [[ "$deploy_router_requested" != "1" ]]; then
      echo "Production router has no service domain; select the router deployment." >&2
      return 1
    fi
    railway domain "${railway_scope[@]}" --service "$router_service" --port 8080 --json >/dev/null
    domains="$(railway domain list "${railway_scope[@]}" --service "$router_service" --json)"
  fi
  router_domain="$(router_public_domain "$domains")"
  router_url="https://$router_domain"
  api_replicas="$(configured_replicas "$api_service")"
  if [[ ! "$api_replicas" =~ ^[1-9][0-9]*$ ]]; then
    echo "Production API must have a positive configured replica count." >&2
    return 1
  fi

  api_variables="$(railway variable list "${railway_scope[@]}" --service "$api_service" --json)"
  if ! jq -e --arg expected "$router_url" '.SCOPE_GIT_PUBLIC_URL == $expected' \
    <<< "$api_variables" >/dev/null; then
    if [[ "$deploy_api_requested" != "1" ]]; then
      echo "Production API Git URL drift requires an API deployment." >&2
      return 1
    fi
    railway variable set "${railway_scope[@]}" --service "$api_service" --skip-deploys \
      "SCOPE_GIT_PUBLIC_URL=$router_url" >/dev/null
  fi

  router_variables="$(railway variable list "${railway_scope[@]}" --service "$router_service" --json)"
  if ! jq -e \
    --arg backend 'scope-api.railway.internal:8080' \
    --arg replicas "$api_replicas" \
    '.SCOPE_REPO_ROUTER_BACKEND == $backend and .SCOPE_REPO_ROUTER_READ_REPLICAS == $replicas' \
    <<< "$router_variables" >/dev/null; then
    if [[ "$deploy_router_requested" != "1" ]]; then
      echo "Production router variable drift requires a router deployment." >&2
      return 1
    fi
    railway variable set "${railway_scope[@]}" --service "$router_service" --skip-deploys \
      'SCOPE_REPO_ROUTER_BACKEND=scope-api.railway.internal:8080' \
      "SCOPE_REPO_ROUTER_READ_REPLICAS=$api_replicas" >/dev/null
  fi

  api_variables="$(railway variable list "${railway_scope[@]}" --service "$api_service" --json)"
  router_variables="$(railway variable list "${railway_scope[@]}" --service "$router_service" --json)"
  jq -e --arg expected "$router_url" '.SCOPE_GIT_PUBLIC_URL == $expected' \
    <<< "$api_variables" >/dev/null
  jq -e \
    --arg backend 'scope-api.railway.internal:8080' \
    --arg replicas "$api_replicas" \
    '.SCOPE_REPO_ROUTER_BACKEND == $backend and .SCOPE_REPO_ROUTER_READ_REPLICAS == $replicas' \
    <<< "$router_variables" >/dev/null

  configure_router_service
}

assert_router_topology() {
  local services_json environment_config_json
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  environment_config_json="$(railway environment config --environment "$environment" --json)"
  SERVICES_JSON="$services_json" \
    ENVIRONMENT_CONFIG_JSON="$environment_config_json" \
    ROUTER_SERVICE_ID="$router_service" \
    ROUTER_GROUP_ID="$router_group" \
    ROUTER_REGION="$api_region" \
    node -e '
const services = JSON.parse(process.env.SERVICES_JSON || "[]");
const environmentConfig = JSON.parse(process.env.ENVIRONMENT_CONFIG_JSON || "{}");
const router = services.find(({id}) => id === process.env.ROUTER_SERVICE_ID);
const replicas = router?.replicas || {};
const liveRegion = router?.regions?.find(({name}) => name === process.env.ROUTER_REGION);
const storedRegions = environmentConfig.services?.[process.env.ROUTER_SERVICE_ID]
  ?.deploy?.multiRegionConfig || {};
const activeStoredRegions = Object.entries(storedRegions).filter(
  ([, config]) => config && Number(config.numReplicas) > 0,
);
if (environmentConfig.services?.[process.env.ROUTER_SERVICE_ID]?.groupId !== process.env.ROUTER_GROUP_ID ||
    router?.status !== "SUCCESS" || router.deploymentStopped === true ||
    replicas.configured !== 1 || replicas.running !== 1 || (replicas.crashed || 0) !== 0 ||
    liveRegion?.configured !== 1 || activeStoredRegions.length !== 1 ||
    activeStoredRegions[0][0] !== process.env.ROUTER_REGION ||
    Number(activeStoredRegions[0][1].numReplicas) !== 1) {
  console.error("Production router topology does not match the reviewed region and replica count.");
  process.exit(1);
}
'
}

service_has_deployment_history() {
  local deployments_json
  deployments_json="$(railway deployment list "${railway_scope[@]}" --service "$1" --limit 1 --json)"
  DEPLOYMENTS_JSON="$deployments_json" node -e '
const deployments = JSON.parse(process.env.DEPLOYMENTS_JSON || "[]");
process.exit(Array.isArray(deployments) && deployments.length > 0 ? 0 : 1);
'
}

deployment_id() {
  local line status running crashed configured stopped id
  line="$(service_state_line "$1")"
  IFS=$'\t' read -r status running crashed configured stopped id <<< "$line"
  if [[ -z "$id" ]]; then
    echo "Railway service $1 has no active deployment to control." >&2
    return 1
  fi
  printf '%s\n' "$id"
}

deployment_action() {
  local action="$1"
  local service_name="$2"
  local expected_deployment_id="${3:-}"
  local id request response
  id="$(deployment_id "$service_name")"
  if [[ -n "$expected_deployment_id" && "$id" != "$expected_deployment_id" ]]; then
    echo "Refusing to $action $service_name: active deployment $id does not match expected deployment $expected_deployment_id." >&2
    return 1
  fi
  request="$(
    DEPLOYMENT_ACTION="$action" \
      DEPLOYMENT_ID="$id" \
      node -e '
const action = process.env.DEPLOYMENT_ACTION;
if (action !== "Stop" && action !== "Restart") process.exit(2);
console.log(JSON.stringify({
  query: `mutation deployment${action}($id: String!) {
    deployment${action}(id: $id)
  }`,
  variables: {id: process.env.DEPLOYMENT_ID},
}));
'
  )"
  response="$(
    printf 'Authorization: Bearer %s\nContent-Type: application/json\n' "$railway_api_token" \
      | curl --silent --show-error --fail-with-body \
        --request POST \
        --url https://backboard.railway.com/graphql/v2 \
        --header @- \
        --data-binary "$request"
  )"
  RAILWAY_GRAPHQL_RESPONSE="$response" DEPLOYMENT_ACTION="$action" node -e '
const response = JSON.parse(process.env.RAILWAY_GRAPHQL_RESPONSE || "{}");
const field = `deployment${process.env.DEPLOYMENT_ACTION}`;
if (response.data?.[field] !== true) {
  const messages = Array.isArray(response.errors)
    ? response.errors.map(({message}) => message).filter(Boolean).join("; ")
    : "";
  console.error(`Railway ${field} mutation failed${messages ? `: ${messages}` : "."}`);
  process.exit(1);
}
'
}

restart_service() {
  local service_name="$1"
  local expected_deployment_id="$2"
  deployment_action Restart "$service_name" "$expected_deployment_id"
  wait_for_service_health "$service_name" "$expected_deployment_id"
}
