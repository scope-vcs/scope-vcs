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
const paths = {"scope-api":"api","scope-worker":"worker","scope-cache-service":"cache-service","scope-repo-router":"repo-router","scope-media":"media-service","scope-media-worker":"media-service","scope-web":"web"};
const instances = states.map(s => {
  const deploy = JSON.parse(fs.readFileSync(`${paths[s.id]}/railway.json`,"utf8")).deploy;
  const deployment = {id:s.deploymentId,status:s.status,deploymentStopped:s.deploymentStopped,
    meta:{serviceManifest:{deploy:{...deploy,numReplicas:s.replicas?.configured}}},
    instances:[...Array.from({length:s.replicas?.running||0},()=>({status:"RUNNING"})),...Array.from({length:s.replicas?.crashed||0},()=>({status:"CRASHED"}))]};
  return {node:{serviceId:s.id,serviceName:s.name,numReplicas:s.replicas?.configured,latestDeployment:deployment,activeDeployments:s.deploymentId ? [deployment] : []}};
});
console.log(JSON.stringify({id:"project-test",environments:{edges:[{node:{id:"production",name:"production",serviceInstances:{edges:instances}}}]},services:{edges:[...Object.entries(paths).map(([id])=>({node:{id,name:({"scope-worker":"scope-run-worker","scope-cache-service":"scope-cache","scope-repo-router":"scope-git-router","scope-media":"scope-media-api"})[id] || id}})),{node:{id:"scope-postgres",name:"scope-postgres"}}]}}));
'
  exit 0
fi

if [[ "$1 $2" == "environment config" ]]; then
  stored_api_region="${FAKE_STORED_API_REGION:-us-east4-eqdc4a}"
  router_config='{}'
  if [[ "${FAKE_ROUTER_CONFIGURED:-1}" == "1" || -f "$FAKE_RAILWAY_STATE/router-scale" ]]; then
    router_config='{"groupId":"runtime-group","deploy":{"multiRegionConfig":{"us-east4-eqdc4a":{"numReplicas":1}}}}'
  fi
  printf '{"services":{"scope-api":{"deploy":{"multiRegionConfig":{"%s":{"numReplicas":1}}}},"scope-worker":{"deploy":{"multiRegionConfig":{"us-east4-eqdc4a":{"numReplicas":1}}}},"scope-media":{"deploy":{"multiRegionConfig":{"us-east4-eqdc4a":{"numReplicas":1}}}},"scope-media-worker":{"deploy":{"multiRegionConfig":{"us-east4-eqdc4a":{"numReplicas":1}}}},"scope-web":{"deploy":{"multiRegionConfig":{"us-east4-eqdc4a":{"numReplicas":1}}}},"scope-repo-router":%s}}\n' "$stored_api_region" "$router_config"
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
    case "$component" in
      worker) component=run-worker ;;
      repo-router) component=git-router ;;
      media) component=media-api ;;
    esac
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
  history_only=0
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "--service" ]]; then service="$2"; shift 2
    elif [[ "$1" == "--limit" && "$2" == 1 ]]; then history_only=1; shift 2
    else shift; fi
  done
  if [[ "$history_only" == 1 && "$service" == "${FAKE_HISTORY_FAILURE_SERVICE:-}" ]]; then
    echo '[]'
    exit 73
  fi
  if [[ -f "$FAKE_RAILWAY_STATE/gate-${service}" ]]; then
    jq -cn --arg service "$service" --arg image "$(cat "$FAKE_RAILWAY_STATE/gate-${service}")" \
      '[{id:("gate-"+$service),serviceId:$service,status:"SUCCESS",deploymentStopped:false,createdAt:"2026-01-01T00:00:00Z",meta:{serviceManifest:{source:{image:$image},deploy:{startCommand:"/app/bin/scope-maintenance serve",healthcheckPath:"/readyz"}}}},{id:("old-"+$service),serviceId:$service,status:"REMOVED",deploymentStopped:true}]'
    exit 0
  fi
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
      --argjson had_gate "$([[ -f "$FAKE_RAILWAY_STATE/gate-history-${service}" ]] && echo true || echo false)" \
      '[{id:$id,serviceId:$service,status:"SUCCESS",createdAt:"2026-01-01T00:00:00Z",meta:{image:$image,imageDigest:($image | split("@") | .[1] // "")}},{id:("old-"+$service),serviceId:$service,status:"REMOVED"}] + if $had_gate then [{id:("gate-"+$service),serviceId:$service,status:"REMOVED",deploymentStopped:true}] else [] end'
  else
    printf '[{"id":"%s","status":"SUCCESS","createdAt":"2026-01-01T00:00:00Z"}]\n' "$id"
  fi
  exit 0
fi

if [[ "$1 $2" == "service list" ]]; then
  node -e '
const { existsSync } = require("node:fs");
const env = process.env;
const marked = (kind, id) => existsSync(`${env.FAKE_RAILWAY_STATE}/${kind}-${id}`);
const region = "us-east4-eqdc4a";
const names = {
  "scope-api": "scope-api", "scope-worker": "scope-run-worker", "scope-cache-service": "scope-cache",
  "scope-media": "scope-media-api", "scope-media-worker": "scope-media-worker",
  "scope-repo-router": "scope-git-router", "scope-web": "scope-web",
};
const replicas = (configured = 1, running = configured, crashed = 0) => ({
  configured, running, crashed, exited: configured - running - crashed, total: configured,
});
function serviceState(id, name) {
  const coreWriter = ["scope-api", "scope-worker", "scope-cache-service"].includes(id);
  const up = marked("up", id);
  const stopped = id !== "scope-repo-router" && marked("stopped", id);
  const state = {
    id, name, status: "SUCCESS", deploymentId: `${up ? "new" : "old"}-${id}`,
    deploymentStopped: stopped, replicas: replicas(),
  };
  if (!["scope-cache-service", "scope-web"].includes(id)) {
    state.regions = [{ name: id === "scope-api" ? env.FAKE_API_REGION || region : region, configured: 1 }];
  }
  if (id !== "scope-web" && marked("no-history", id) && !up) {
    state.deploymentId = null;
    state.replicas = null;
  }
  if (stopped) state.replicas = replicas(1, 0);
  if (up && !stopped && !marked("crashed", id)) {
    state.replicas = replicas(coreWriter ? Number(env.FAKE_NEW_REPLICAS || 1) : 1);
  }
  if (coreWriter && !stopped && marked("crashed", id)) {
    state.status = "CRASHED";
    state.replicas = replicas(1, 0, 1);
  }
  if (coreWriter && !stopped && env.FAKE_DEGRADED_SERVICE === id) {
    state.replicas = replicas(2, 1, 1);
    if (state.regions) state.regions[0].configured = 2;
  }
  if (["scope-api", "scope-worker"].includes(id) && up && env.FAKE_UNHEALTHY_AFTER_UP_SERVICE === id) {
    state.replicas = replicas(2, 1, 1);
    state.regions = [{ name: region, configured: 2 }];
  }
  if (id === "scope-worker" && !stopped && env.FAKE_DEGRADE_WORKER_AFTER_API_STOP === "1" && marked("stopped", "scope-api")) {
    state.status = "CRASHED";
    state.replicas = replicas(1, 0, 1);
  }
  if (marked("gate", id)) Object.assign(state, {
    deploymentId: `gate-${id}`, status: "SUCCESS", deploymentStopped: false, replicas: replicas(),
  });
  return state;
}
const routerExists = env.FAKE_ROUTER_INSTANCE_EXISTS !== "0" || existsSync(`${env.FAKE_RAILWAY_STATE}/router-instance-created`);
console.log(JSON.stringify(Object.entries(names)
  .filter(([id]) => id !== "scope-repo-router" || routerExists)
  .map(([id, name]) => serviceState(id, name))));
'
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
  # Router readiness resolves the API private address; stopped API replicas
  # provide no DNS target for a newly starting router.
  if [[ "$service" == "scope-repo-router" ]] \
    && [[ "$("$0" service list | jq -r '.[] | select(.id == "scope-api") | .replicas.running')" == "0" ]]; then
    echo "Git router readiness failed: API private DNS has no running target." >&2
    exit 1
  fi
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
  rm -f "$FAKE_RAILWAY_STATE/stopped-${service}" "$FAKE_RAILWAY_STATE/gate-${service}"
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

# Gate provider behavior is covered by railway-maintenance-gate.test.mjs. The
# orchestration fixture uses readable non-UUID IDs, so model only its CLI boundary.
real_node="$(command -v node)"
printf '#!/usr/bin/env bash\nif [[ "${1:-}" == */railway-maintenance-gate.mjs ]]; then\n  exec %q "$(dirname "$0")/gate-node.cjs" "$@"\nfi\nexec %q "$@"\n' "$real_node" "$real_node" > "$test_dir/bin/node"
cat > "$test_dir/bin/gate-node.cjs" <<'FAKE'
const fs = require('node:fs');
const args = process.argv.slice(2);
const [script, action, file] = args;
const gate = JSON.parse(fs.readFileSync(file, 'utf8'));
const state = process.env.FAKE_RAILWAY_STATE;
const trace = message => fs.appendFileSync(process.env.FAKE_RAILWAY_TRACE, `${message}\n`);
const marker = name => `${state}/${name}-${gate.serviceId}`;
if (action === 'snapshot') {
  gate.previous = {
    source: { image: `ghcr.io/test/repo/baseline@sha256:${'a'.repeat(64)}` },
    build: { rootDirectory: '/', railwayConfigFile: null, buildCommand: null },
    deploy: { startCommand: '/app/bin/original', healthcheckPath: '/readyz', healthcheckTimeout: 60, preDeployCommand: [] },
  };
  gate.predecessorIds = [`${fs.existsSync(marker('up')) ? 'new' : 'old'}-${gate.serviceId}`];
  gate.phase = 'snapshotted';
  gate.capturedAt = new Date().toISOString();
} else if (action === 'reclose' || action === 'enter') {
  if (!gate.previous) throw new Error('Missing original gate snapshot');
  gate.deploymentId = `gate-${gate.serviceId}`;
  gate.phase = 'active';
  fs.writeFileSync(marker('gate'), gate.image);
  fs.writeFileSync(marker('gate-history'), gate.image);
  const predecessor = `${fs.existsSync(marker('up')) ? 'new' : 'old'}-${gate.serviceId}`;
  if (!fs.existsSync(marker('stopped'))) {
    trace(`graphql stop ${gate.serviceId} ${predecessor}`);
    if (process.env.FAKE_DENY_DEPLOYMENT_ACTION_SERVICE === gate.serviceId) {
      throw new Error('Railway denied predecessor stop');
    }
  }
  fs.writeFileSync(marker('stopped'), '');
  fs.writeFileSync(marker('stop-requested'), '');
  trace(`gate stop-predecessors ${gate.serviceId}`);
} else if (action === 'restore') {
  if (!gate.previous) throw new Error('Missing original gate snapshot');
  gate.phase = 'restored';
  // Restoring config does not reopen service or restart removed predecessors.
} else throw new Error(`Unknown gate action ${action}`);
trace(`gate ${action} ${gate.serviceId}`);
fs.writeFileSync(file, JSON.stringify(gate));
FAKE
chmod +x "$test_dir/bin/node"
