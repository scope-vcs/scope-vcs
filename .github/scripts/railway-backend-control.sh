#!/usr/bin/env bash

validate_production_target() {
  local status_json services_json environment_config_json
  status_json="$(railway status "${railway_scope[@]}" --json)"
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  environment_config_json="$(railway environment config --environment "$environment" --json)"
  RAILWAY_STATUS_JSON="$status_json" RAILWAY_SERVICES_JSON="$services_json" \
    RAILWAY_ENVIRONMENT_CONFIG_JSON="$environment_config_json" \
    node .github/scripts/railway-production-target.mjs production
}

service_state_field() {
  local services_json
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  jq -r --arg service "$1" --arg field "$2" '
    first(.[] | select(.id == $service or .name == $service))
    | {running: (.replicas.running // 0), configured: (.replicas.configured // 0), id: (.deploymentId // "")}[$field]
  ' <<< "$services_json"
}

running_replicas() {
  service_state_field "$1" running || true
}

configured_replicas() {
  service_state_field "$1" configured || true
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
  local services_json
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  jq -e --arg service "$1" 'any(.[]; .id == $service)' <<< "$services_json" >/dev/null
}

within_a_minute() {
  local deadline=$((SECONDS + 60))
  while (( SECONDS < deadline )); do
    "$@" && return 0
    sleep 2
  done
  return 1
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
  if production_service_exists "$router_service"; then
    return 0
  fi
  if [[ "$deploy_router_requested" != "1" ]]; then
    echo "Production router has no service instance; select the router deployment." >&2
    return 1
  fi
  commit_environment_patch "$(
    jq -cn --arg service "$router_service" --arg group "$router_group" \
      '{services: {($service): {isCreated: true, groupId: $group}}}'
  )" "Create production Git router instance"
  within_a_minute production_service_exists "$router_service" && return 0
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
  if router_service_config_matches; then
    return 0
  fi
  if [[ "$deploy_router_requested" != "1" ]]; then
    echo "Production router service configuration drift requires a router deployment." >&2
    return 1
  fi
  commit_environment_patch "$(
    jq -cn --arg service "$router_service" --arg group "$router_group" --arg region "$api_region" \
      '{services: {($service): {groupId: $group, deploy: {multiRegionConfig: {($region): {numReplicas: 1}}}}}}'
  )" "Configure production Git router topology"
  within_a_minute router_service_config_matches && return 0
  echo "Production router service configuration did not converge after update." >&2
  return 1
}

api_git_url_matches() {
  local variables
  variables="$(railway variable list "${railway_scope[@]}" --service "$api_service" --json)" || exit $?
  jq -e --arg expected "$router_url" '.SCOPE_GIT_PUBLIC_URL == $expected' <<< "$variables" >/dev/null
}

router_variables_match() {
  local variables
  variables="$(railway variable list "${railway_scope[@]}" --service "$router_service" --json)" || exit $?
  jq -e --arg backend 'scope-api.railway.internal:8080' --arg replicas "$api_replicas" \
    '.SCOPE_REPO_ROUTER_BACKEND == $backend and .SCOPE_REPO_ROUTER_READ_REPLICAS == $replicas' <<< "$variables" >/dev/null
}

configure_production_router() {
  local domains router_url api_replicas
  domains="$(railway domain list "${railway_scope[@]}" --service "$router_service" --json)"
  if [[ "$(jq '[.domains[]? | select(.type == "service")] | length' <<< "$domains")" == "0" ]]; then
    if [[ "$deploy_router_requested" != "1" ]]; then
      echo "Production router has no service domain; select the router deployment." >&2
      return 1
    fi
    railway domain "${railway_scope[@]}" --service "$router_service" --port 8080 --json >/dev/null
    domains="$(railway domain list "${railway_scope[@]}" --service "$router_service" --json)"
  fi
  router_url="https://$(router_public_domain "$domains")"
  api_replicas="$(configured_replicas "$api_service")"
  if [[ ! "$api_replicas" =~ ^[1-9][0-9]*$ ]]; then
    echo "Production API must have a positive configured replica count." >&2
    return 1
  fi

  if ! api_git_url_matches; then
    if [[ "$deploy_api_requested" != "1" ]]; then
      echo "Production API Git URL drift requires an API deployment." >&2
      return 1
    fi
    railway variable set "${railway_scope[@]}" --service "$api_service" --skip-deploys \
      "SCOPE_GIT_PUBLIC_URL=$router_url" >/dev/null
  fi

  if ! router_variables_match; then
    if [[ "$deploy_router_requested" != "1" ]]; then
      echo "Production router variable drift requires a router deployment." >&2
      return 1
    fi
    railway variable set "${railway_scope[@]}" --service "$router_service" --skip-deploys \
      'SCOPE_REPO_ROUTER_BACKEND=scope-api.railway.internal:8080' \
      "SCOPE_REPO_ROUTER_READ_REPLICAS=$api_replicas" >/dev/null
  fi

  api_git_url_matches
  router_variables_match
  configure_router_service
}

assert_router_topology() {
  local services_json environment_config_json
  services_json="$(railway service list "${railway_scope[@]}" --json)"
  environment_config_json="$(railway environment config --environment "$environment" --json)"
  RAILWAY_SERVICES_JSON="$services_json" RAILWAY_ENVIRONMENT_CONFIG_JSON="$environment_config_json" \
    node .github/scripts/railway-production-target.mjs router
}

service_has_deployment_history() {
  local deployments_json
  deployments_json="$(railway_read deployment list "${railway_scope[@]}" --service "$1" --limit 1 --json)" || return $?
  jq -er 'if type == "array" then (if length > 0 then 1 else 0 end)
    else error("Railway deployment inventory must be an array.") end' <<< "$deployments_json"
}

deployment_id() {
  local id
  id="$(service_state_field "$1" id)"
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
  request="$(jq -cn --arg action "$action" --arg id "$id" \
    '{query: "mutation deployment\($action)($id: String!) { deployment\($action)(id: $id) }", variables: {id: $id}}')"
  response="$(
    printf 'Authorization: Bearer %s\nContent-Type: application/json\n' "$railway_api_token" \
      | curl --silent --show-error --fail-with-body \
        --request POST \
        --url https://backboard.railway.com/graphql/v2 \
        --header @- \
        --data-binary "$request"
  )"
  jq -e --arg field "deployment$action" '.data[$field] == true' <<< "$response" >/dev/null 2>&1 || {
    echo "Railway deployment$action mutation failed$(jq -r '[.errors[]?.message // empty] | join("; ") | if . == "" then "." else ": \(.)" end' <<< "$response" 2>/dev/null || echo .)" >&2
    return 1
  }
}

restart_service() {
  local service_name="$1"
  local expected_deployment_id="$2"
  deployment_action Restart "$service_name" "$expected_deployment_id"
  wait_for_service_health "$service_name" "$expected_deployment_id"
}
