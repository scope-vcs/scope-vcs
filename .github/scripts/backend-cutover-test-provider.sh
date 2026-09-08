#!/usr/bin/env bash
# Provider doubles shared by the backend cutover regression scenarios.
cat > "$test_dir/bin/railway" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_RAILWAY_TRACE"

if [[ "$1" == "status" ]]; then
  states="$("$0" service list)"
  STATES="$states" node -e '
const fs = require("node:fs");
const states = JSON.parse(process.env.STATES);
const paths = {"scope-api":"api","scope-worker":"worker","scope-cache-service":"cache-service","scope-repo-router":"repo-router","scope-media":"media-service","scope-media-worker":"media-service"};
const instances = states.map(s => {
  const deploy = JSON.parse(fs.readFileSync(`${paths[s.id]}/railway.json`,"utf8")).deploy;
  const deployment = {id:s.deploymentId,status:s.status,deploymentStopped:s.deploymentStopped,
    meta:{serviceManifest:{deploy:{...deploy,numReplicas:s.replicas?.configured}}},
    instances:[...Array.from({length:s.replicas?.running||0},()=>({status:"RUNNING"})),...Array.from({length:s.replicas?.crashed||0},()=>({status:"CRASHED"}))]};
  return {node:{serviceId:s.id,serviceName:s.name,numReplicas:s.replicas?.configured,latestDeployment:deployment,activeDeployments:s.deploymentId ? [deployment] : []}};
});
console.log(JSON.stringify({id:"project-test",environments:{edges:[{node:{id:"production",name:"production",serviceInstances:{edges:instances}}}]},services:{edges:[...Object.entries(paths).map(([id])=>({node:{id,name:id}})),{node:{id:"scope-postgres",name:"scope-postgres"}}]}}));
'
  exit 0
fi

if [[ "$1 $2" == "environment config" ]]; then
  stored_api_region="${FAKE_STORED_API_REGION:-us-east4-eqdc4a}"
  router_config='{}'
  if [[ "${FAKE_ROUTER_CONFIGURED:-1}" == "1" || -f "$FAKE_RAILWAY_STATE/router-scale" ]]; then
    router_config='{"groupId":"runtime-group","deploy":{"multiRegionConfig":{"us-east4-eqdc4a":{"numReplicas":1}}}}'
  fi
  printf '{"services":{"scope-api":{"deploy":{"multiRegionConfig":{"%s":{"numReplicas":1}}}},"scope-worker":{"deploy":{"multiRegionConfig":{"us-east4-eqdc4a":{"numReplicas":1}}}},"scope-media":{"deploy":{"multiRegionConfig":{"us-east4-eqdc4a":{"numReplicas":1}}}},"scope-media-worker":{"deploy":{"multiRegionConfig":{"us-east4-eqdc4a":{"numReplicas":1}}}},"scope-repo-router":%s}}\n' "$stored_api_region" "$router_config"
  exit 0
fi

if [[ "$1" == "api" && "$2" == *"SelectedConfig"* ]]; then
  cat >/dev/null
  node -e '
const fs = require("node:fs");
const directory = process.env.FAKE_RAILWAY_STATE;
const services = Object.fromEntries(fs.readdirSync(directory).filter(f=>f.startsWith("image-")).map(f=>[f.slice(6),{source:{image:fs.readFileSync(`${directory}/${f}`,"utf8").trim(),repo:null}}]));
console.log(JSON.stringify({data:{environment:{config:{services}}}}));
'
  exit 0
fi

if [[ "$1" == "api" && "$2" == *"serviceInstance"* ]]; then
  variables="$(cat)"
  service="$(jq -r .serviceId <<< "$variables")"
  if [[ "$2" == *"serviceInstanceUpdate("* ]]; then
    jq -r .input.source.image <<< "$variables" > "$FAKE_RAILWAY_STATE/image-$service"
    echo '{"data":{"serviceInstanceUpdate":true}}'
  elif [[ "$2" == *"serviceInstance("* ]]; then
    jq -cn --arg image "$(cat "$FAKE_RAILWAY_STATE/image-$service")" '{data:{serviceInstance:{source:{image:$image,repo:null}}}}'
  else
    component="${service#scope-}"
    [[ "$component" != cache-service ]] || component=cache
    [[ "$component" != repo-router ]] || component=router
    deployed="$(FAKE_IMMUTABLE_ACTIVATION=1 "$0" up "$FAKE_UPLOAD_ROOT/$component" --service "$service")"
    jq -c '{data:{serviceInstanceDeployV2:.deploymentId}}' <<< "$deployed"
  fi
  exit 0
fi

if [[ "$1" == "api" ]]; then
  [[ -z "${RAILWAY_TOKEN:-}" ]]
  [[ "${RAILWAY_API_TOKEN:-}" == "token-graphql" ]]
  variables=""
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "--variables" ]]; then variables="$2"; shift 2; else shift; fi
  done
  action="$(
    VARIABLES_JSON="$variables" node -e '
const variables = JSON.parse(process.env.VARIABLES_JSON || "{}");
const services = variables.patch?.services || {};
const ids = Object.keys(services);
const config = services["scope-repo-router"] || {};
const validBase = variables.environmentId === "production" && ids.length === 1 &&
  config.groupId === "runtime-group";
if (validBase && config.isCreated === true) console.log("create-instance");
else if (validBase && config.deploy?.multiRegionConfig?.["us-east4-eqdc4a"]?.numReplicas === 1) {
  console.log("configure-scale");
} else console.log("invalid");
'
  )"
  [[ "$action" != "invalid" ]]
  if [[ "$action" == "create-instance" ]]; then
    touch "$FAKE_RAILWAY_STATE/router-instance-created"
  else
    touch "$FAKE_RAILWAY_STATE/router-scale"
  fi
  printf 'graphql %s scope-repo-router\n' "$action" >> "$FAKE_RAILWAY_TRACE"
  echo '{"data":{"environmentPatchCommit":"router-config-commit"}}'
  exit 0
fi

if [[ "$1" == "run" ]]; then
  service=""
  while [[ "$1" != "--" ]]; do
    if [[ "$1" == "--service" ]]; then
      service="$2"
      shift 2
    else
      shift
    fi
  done
  shift
  if [[ "$service" == "scope-postgres" ]]; then
    DATABASE_PUBLIC_URL="postgres://public-database.test/scope" "$@"
  elif [[ "$service" == "scope-api" ]]; then
    [[ "${SCOPE_MAINTENANCE_DATABASE_URL:-}" == "postgres://public-database.test/scope" ]]
    [[ -n "${SCOPE_MAINTENANCE_DATA_DIR:-}" ]]
    "$@"
  else
    exit 2
  fi
  exit $?
fi

if [[ "$1 $2" == "domain list" ]]; then
  if [[ "${FAKE_ROUTER_INSTANCE_EXISTS:-1}" == "0" \
    && ! -f "$FAKE_RAILWAY_STATE/router-instance-created" ]]; then
    echo "ServiceInstance not found" >&2
    exit 1
  fi
  if [[ "${FAKE_ROUTER_DOMAIN_STATE:-valid}" == "invalid" ]]; then
    echo '{"domains":[{"domain":"scope-repo-router-production.test","type":"service","syncStatus":"ACTIVE","targetPort":9090}]}'
  elif [[ "${FAKE_ROUTER_CONFIGURED:-1}" == "1" || -f "$FAKE_RAILWAY_STATE/router-domain" ]]; then
    echo '{"domains":[{"domain":"scope-repo-router-production.test","type":"service","syncStatus":"ACTIVE","targetPort":8080}]}'
  else
    echo '{"domains":[]}'
  fi
  exit 0
fi

if [[ "$1" == "domain" ]]; then
  touch "$FAKE_RAILWAY_STATE/router-domain"
  echo '{"domain":"scope-repo-router-production.test"}'
  exit 0
fi

if [[ "$1 $2" == "variable list" ]]; then
  service=""
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "--service" ]]; then service="$2"; shift 2; else shift; fi
  done
  if [[ "$service" == "scope-postgres" ]]; then
    echo '{"DATABASE_PUBLIC_URL":"postgres://public-database.test/scope"}'
  elif [[ "$service" == "scope-api" ]]; then
    if [[ "${FAKE_ROUTER_CONFIGURED:-1}" == "1" || -f "$FAKE_RAILWAY_STATE/api-router-variable" ]]; then
      echo '{"SCOPE_GIT_PUBLIC_URL":"https://scope-repo-router-production.test"}'
    else
      echo '{}'
    fi
  elif [[ "$service" == "scope-repo-router" ]]; then
    if [[ "${FAKE_ROUTER_CONFIGURED:-1}" == "1" || -f "$FAKE_RAILWAY_STATE/router-variables" ]]; then
      echo '{"SCOPE_REPO_ROUTER_BACKEND":"scope-api.railway.internal:8080","SCOPE_REPO_ROUTER_READ_REPLICAS":"1"}'
    else
      echo '{}'
    fi
  else
    exit 2
  fi
  exit 0
fi

if [[ "$1 $2" == "variable set" ]]; then
  service=""
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "--service" ]]; then service="$2"; shift 2; else shift; fi
  done
  [[ "$service" == "scope-api" ]] && touch "$FAKE_RAILWAY_STATE/api-router-variable"
  [[ "$service" == "scope-repo-router" ]] && touch "$FAKE_RAILWAY_STATE/router-variables"
  exit 0
fi

if [[ "$1 $2" == "deployment list" ]]; then
  service=""
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "--service" ]]; then service="$2"; shift 2; else shift; fi
  done
  if [[ -f "$FAKE_RAILWAY_STATE/no-history-${service}" && ! -f "$FAKE_RAILWAY_STATE/up-${service}" ]]; then
    echo '[]'
    exit 0
  fi
  id="old-${service}"
  [[ -f "$FAKE_RAILWAY_STATE/up-${service}" ]] && id="new-${service}"
  if [[ -f "$FAKE_RAILWAY_STATE/skipped-${service}" ]]; then
    printf '[{"id":"skip-%s","status":"SKIPPED","createdAt":"2026-01-02T00:00:00Z","meta":{"skippedReason":"identical"}},{"id":"new-%s","status":"SUCCESS","createdAt":"2026-01-01T00:00:00Z"}]\n' "$service" "$service"
    exit 0
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/crashed-${service}" ]]; then
    printf '[{"id":"%s","status":"CRASHED","createdAt":"2026-01-02T00:00:00Z"}]\n' "$id"
    exit 0
  fi
  if [[ "$id" == new-* ]]; then
    image=""
    [[ ! -f "$FAKE_RAILWAY_STATE/image-$service" ]] || image="$(cat "$FAKE_RAILWAY_STATE/image-$service")"
    jq -cn --arg id "$id" --arg service "$service" --arg image "$image" \
      '[{id:$id,status:"SUCCESS",createdAt:"2026-01-01T00:00:00Z",meta:{image:$image,imageDigest:($image | split("@") | .[1] // "")}},{id:("old-"+$service),status:"REMOVED"}]'
  else
    printf '[{"id":"%s","status":"SUCCESS","createdAt":"2026-01-01T00:00:00Z"}]\n' "$id"
  fi
  exit 0
fi

if [[ "$1 $2" == "service list" ]]; then
  api_region="${FAKE_API_REGION:-us-east4-eqdc4a}"
  api_deployment='"old-scope-api"'
  worker_deployment='"old-scope-worker"'
  cache_deployment='"old-scope-cache-service"'
  router_deployment='"old-scope-repo-router"'
  media_deployment='"old-scope-media"'
  media_worker_deployment='"old-scope-media-worker"'
  api_status=SUCCESS
  worker_status=SUCCESS
  cache_status=SUCCESS
  router_status=SUCCESS
  media_status=SUCCESS
  media_worker_status=SUCCESS
  api_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  worker_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  cache_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  router_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  media_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  media_worker_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  api_stopped=false
  worker_stopped=false
  cache_stopped=false
  router_stopped=false
  media_stopped=false
  media_worker_stopped=false
  api_regions="[{\"name\":\"${api_region}\",\"configured\":1}]"
  worker_regions='[{"name":"us-east4-eqdc4a","configured":1}]'
  router_regions='[{"name":"us-east4-eqdc4a","configured":1}]'
  media_regions='[{"name":"us-east4-eqdc4a","configured":1}]'
  media_worker_regions='[{"name":"us-east4-eqdc4a","configured":1}]'
  if [[ -f "$FAKE_RAILWAY_STATE/no-history-scope-api" && ! -f "$FAKE_RAILWAY_STATE/up-scope-api" ]]; then
    api_deployment=null
    api_replicas=null
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/no-history-scope-worker" && ! -f "$FAKE_RAILWAY_STATE/up-scope-worker" ]]; then
    worker_deployment=null
    worker_replicas=null
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/no-history-scope-cache-service" && ! -f "$FAKE_RAILWAY_STATE/up-scope-cache-service" ]]; then
    cache_deployment=null
    cache_replicas=null
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/no-history-scope-repo-router" && ! -f "$FAKE_RAILWAY_STATE/up-scope-repo-router" ]]; then
    router_deployment=null
    router_replicas=null
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/no-history-scope-media" && ! -f "$FAKE_RAILWAY_STATE/up-scope-media" ]]; then
    media_deployment=null
    media_replicas=null
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/no-history-scope-media-worker" && ! -f "$FAKE_RAILWAY_STATE/up-scope-media-worker" ]]; then
    media_worker_deployment=null
    media_worker_replicas=null
  fi
  [[ -f "$FAKE_RAILWAY_STATE/up-scope-api" ]] && api_deployment='"new-scope-api"'
  [[ -f "$FAKE_RAILWAY_STATE/up-scope-worker" ]] && worker_deployment='"new-scope-worker"'
  [[ -f "$FAKE_RAILWAY_STATE/up-scope-cache-service" ]] && cache_deployment='"new-scope-cache-service"'
  [[ -f "$FAKE_RAILWAY_STATE/up-scope-repo-router" ]] && router_deployment='"new-scope-repo-router"'
  [[ -f "$FAKE_RAILWAY_STATE/up-scope-media" ]] && media_deployment='"new-scope-media"'
  [[ -f "$FAKE_RAILWAY_STATE/up-scope-media-worker" ]] && media_worker_deployment='"new-scope-media-worker"'
  if [[ -f "$FAKE_RAILWAY_STATE/stopped-scope-api" ]]; then
    api_stopped=true
    api_replicas='{"configured":1,"running":0,"crashed":0,"exited":1,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/stopped-scope-worker" ]]; then
    worker_stopped=true
    worker_replicas='{"configured":1,"running":0,"crashed":0,"exited":1,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/stopped-scope-cache-service" ]]; then
    cache_stopped=true
    cache_replicas='{"configured":1,"running":0,"crashed":0,"exited":1,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/stopped-scope-media" ]]; then
    media_stopped=true
    media_replicas='{"configured":1,"running":0,"crashed":0,"exited":1,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/stopped-scope-media-worker" ]]; then
    media_worker_stopped=true
    media_worker_replicas='{"configured":1,"running":0,"crashed":0,"exited":1,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/up-scope-api" && "$api_stopped" == "false" && ! -f "$FAKE_RAILWAY_STATE/crashed-scope-api" ]]; then
    api_replicas="{\"configured\":${FAKE_NEW_REPLICAS:-1},\"running\":${FAKE_NEW_REPLICAS:-1},\"crashed\":0,\"exited\":0,\"total\":${FAKE_NEW_REPLICAS:-1}}"
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/up-scope-worker" && "$worker_stopped" == "false" && ! -f "$FAKE_RAILWAY_STATE/crashed-scope-worker" ]]; then
    worker_replicas="{\"configured\":${FAKE_NEW_REPLICAS:-1},\"running\":${FAKE_NEW_REPLICAS:-1},\"crashed\":0,\"exited\":0,\"total\":${FAKE_NEW_REPLICAS:-1}}"
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/up-scope-cache-service" && "$cache_stopped" == "false" && ! -f "$FAKE_RAILWAY_STATE/crashed-scope-cache-service" ]]; then
    cache_replicas="{\"configured\":${FAKE_NEW_REPLICAS:-1},\"running\":${FAKE_NEW_REPLICAS:-1},\"crashed\":0,\"exited\":0,\"total\":${FAKE_NEW_REPLICAS:-1}}"
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/up-scope-repo-router" && ! -f "$FAKE_RAILWAY_STATE/crashed-scope-repo-router" ]]; then
    router_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/up-scope-media" && "$media_stopped" == "false" && ! -f "$FAKE_RAILWAY_STATE/crashed-scope-media" ]]; then
    media_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/up-scope-media-worker" && "$media_worker_stopped" == "false" && ! -f "$FAKE_RAILWAY_STATE/crashed-scope-media-worker" ]]; then
    media_worker_replicas='{"configured":1,"running":1,"crashed":0,"exited":0,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/crashed-scope-api" && "$api_stopped" == "false" ]]; then
    api_status=CRASHED
    api_replicas='{"configured":1,"running":0,"crashed":1,"exited":0,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/crashed-scope-worker" && "$worker_stopped" == "false" ]]; then
    worker_status=CRASHED
    worker_replicas='{"configured":1,"running":0,"crashed":1,"exited":0,"total":1}'
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/crashed-scope-cache-service" && "$cache_stopped" == "false" ]]; then
    cache_status=CRASHED
    cache_replicas='{"configured":1,"running":0,"crashed":1,"exited":0,"total":1}'
  fi
  if [[ "${FAKE_DEGRADED_SERVICE:-}" == "scope-api" && "$api_stopped" == "false" ]]; then
    api_replicas='{"configured":2,"running":1,"crashed":1,"exited":0,"total":2}'
    api_regions="[{\"name\":\"${api_region}\",\"configured\":2}]"
  fi
  if [[ "${FAKE_DEGRADED_SERVICE:-}" == "scope-worker" && "$worker_stopped" == "false" ]]; then
    worker_replicas='{"configured":2,"running":1,"crashed":1,"exited":0,"total":2}'
    worker_regions='[{"name":"us-east4-eqdc4a","configured":2}]'
  fi
  if [[ "${FAKE_DEGRADED_SERVICE:-}" == "scope-cache-service" && "$cache_stopped" == "false" ]]; then
    cache_replicas='{"configured":2,"running":1,"crashed":1,"exited":0,"total":2}'
  fi
  if [[ "${FAKE_UNHEALTHY_AFTER_UP_SERVICE:-}" == "scope-api" \
    && -f "$FAKE_RAILWAY_STATE/up-scope-api" ]]; then
    api_replicas='{"configured":2,"running":1,"crashed":1,"exited":0,"total":2}'
    api_regions='[{"name":"us-east4-eqdc4a","configured":2}]'
  fi
  if [[ "${FAKE_UNHEALTHY_AFTER_UP_SERVICE:-}" == "scope-worker" \
    && -f "$FAKE_RAILWAY_STATE/up-scope-worker" ]]; then
    worker_replicas='{"configured":2,"running":1,"crashed":1,"exited":0,"total":2}'
    worker_regions='[{"name":"us-east4-eqdc4a","configured":2}]'
  fi
  if [[ "${FAKE_DEGRADE_WORKER_AFTER_API_STOP:-0}" == "1" \
    && -f "$FAKE_RAILWAY_STATE/stopped-scope-api" && "$worker_stopped" == "false" ]]; then
    worker_status=CRASHED
    worker_replicas='{"configured":1,"running":0,"crashed":1,"exited":0,"total":1}'
  fi
  router_json=""
  if [[ "${FAKE_ROUTER_INSTANCE_EXISTS:-1}" == "1" \
    || -f "$FAKE_RAILWAY_STATE/router-instance-created" ]]; then
    router_json=",{\"id\":\"scope-repo-router\",\"name\":\"scope-repo-router\",\"status\":\"${router_status}\",\"deploymentId\":${router_deployment},\"deploymentStopped\":${router_stopped},\"replicas\":${router_replicas},\"regions\":${router_regions}}"
  fi
  printf '[{"id":"scope-api","name":"scope-api","status":"%s","deploymentId":%s,"deploymentStopped":%s,"replicas":%s,"regions":%s},{"id":"scope-worker","name":"scope-worker","status":"%s","deploymentId":%s,"deploymentStopped":%s,"replicas":%s,"regions":%s},{"id":"scope-cache-service","name":"scope-cache-service","status":"%s","deploymentId":%s,"deploymentStopped":%s,"replicas":%s},{"id":"scope-media","name":"scope-media","status":"%s","deploymentId":%s,"deploymentStopped":%s,"replicas":%s,"regions":%s},{"id":"scope-media-worker","name":"scope-media-worker","status":"%s","deploymentId":%s,"deploymentStopped":%s,"replicas":%s,"regions":%s}%s]\n' "$api_status" "$api_deployment" "$api_stopped" "$api_replicas" "$api_regions" "$worker_status" "$worker_deployment" "$worker_stopped" "$worker_replicas" "$worker_regions" "$cache_status" "$cache_deployment" "$cache_stopped" "$cache_replicas" "$media_status" "$media_deployment" "$media_stopped" "$media_replicas" "$media_regions" "$media_worker_status" "$media_worker_deployment" "$media_worker_stopped" "$media_worker_replicas" "$media_worker_regions" "$router_json"
  exit 0
fi

if [[ "$1 $2" == "service scale" ]]; then
  echo "project-token service scale must not be used" >&2
  exit 97
fi

if [[ "$1" == "up" ]]; then
  service=""
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "--service" ]]; then service="$2"; shift 2; else shift; fi
  done
  [[ "${FAKE_FAIL_UP_SERVICE:-}" == "$service" ]] && exit 1
  if [[ "${FAKE_SKIP_UP_SERVICE:-}" == "$service" ]]; then
    touch "$FAKE_RAILWAY_STATE/skipped-${service}"
    printf '{"deploymentId":"skip-%s"}\n' "$service"
    exit 0
  fi
  if [[ "${FAKE_CRASH_UP_SERVICE:-}" == "$service" ]]; then
    touch "$FAKE_RAILWAY_STATE/up-${service}" "$FAKE_RAILWAY_STATE/crashed-${service}"
    rm -f "$FAKE_RAILWAY_STATE/stopped-${service}"
    printf '{"deploymentId":"new-%s"}\n' "$service"
    exit 0
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/up-${service}" && "${FAKE_IMMUTABLE_ACTIVATION:-0}" != "1" ]]; then
    touch "$FAKE_RAILWAY_STATE/skipped-${service}"
    printf '{"deploymentId":"skip-%s"}\n' "$service"
    exit 0
  fi
  touch "$FAKE_RAILWAY_STATE/up-${service}"
  rm -f "$FAKE_RAILWAY_STATE/stopped-${service}"
  printf '{"deploymentId":"new-%s"}\n' "$service"
  exit 0
fi

echo "unexpected fake Railway invocation: $*" >&2
exit 2
FAKE
chmod +x "$test_dir/bin/railway"

cat > "$test_dir/bin/curl" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
headers="$(cat)"
[[ "$headers" == *"Authorization: Bearer token-graphql"* ]]
[[ "$headers" == *"Content-Type: application/json"* ]]

request=""
while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --data-binary) request="$2"; shift 2 ;;
    *) shift ;;
  esac
done
read -r action deployment_id < <(
  REQUEST_JSON="$request" node -e '
const request = JSON.parse(process.env.REQUEST_JSON || "{}");
const match = request.query?.match(/deployment(Stop|Restart)/);
console.log(`${match?.[1]?.toLowerCase() || ""} ${request.variables?.id || ""}`);
'
)
service="${deployment_id#old-}"
service="${service#new-}"
printf 'graphql %s %s %s\n' "$action" "$service" "$deployment_id" >> "$FAKE_RAILWAY_TRACE"
if [[ "${FAKE_DENY_DEPLOYMENT_ACTION_SERVICE:-}" == "$service" ]]; then
  echo '{"errors":[{"message":"permission denied"}]}'
  exit 0
fi
if [[ "$action" == "stop" ]]; then
  touch "$FAKE_RAILWAY_STATE/stop-requested-${service}"
  if [[ "${FAKE_STALE_STOP_STATUS:-0}" != "1" ]]; then
    touch "$FAKE_RAILWAY_STATE/stopped-${service}"
  fi
  echo '{"data":{"deploymentStop":true}}'
elif [[ "$action" == "restart" ]]; then
  rm -f "$FAKE_RAILWAY_STATE/stopped-${service}" \
    "$FAKE_RAILWAY_STATE/stop-requested-${service}" \
    "$FAKE_RAILWAY_STATE/crashed-${service}"
  echo '{"data":{"deploymentRestart":true}}'
else
  exit 2
fi
FAKE
chmod +x "$test_dir/bin/curl"
