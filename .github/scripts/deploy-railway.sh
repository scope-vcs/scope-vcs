#!/usr/bin/env bash
set -euo pipefail

# Prepared activations (SCOPE_PREPARED_RELEASE_PATH) deploy an immutable image and
# take no upload root; source uploads require the directory to send to Railway.
service_name="${1:?usage: deploy-railway.sh <service-name> [upload-root]}"
upload_root="${2:-}"

if [[ -z "${RAILWAY_API_TOKEN:-}" && -z "${RAILWAY_TOKEN:-}" ]]; then
  echo "Set RAILWAY_API_TOKEN or RAILWAY_TOKEN before deploying ${service_name}."
  exit 1
fi

if [[ -n "${RAILWAY_API_TOKEN:-}" && -n "${RAILWAY_TOKEN:-}" ]]; then
  echo "Set only one Railway token type before deploying ${service_name}."
  exit 1
fi

if [ -z "${RAILWAY_PROJECT_ID:-}" ]; then
  echo "Set RAILWAY_PROJECT_ID before deploying ${service_name}."
  exit 1
fi

railway_environment="${SCOPE_RAILWAY_ENVIRONMENT_ID:?SCOPE_RAILWAY_ENVIRONMENT_ID is required}"
deployment_component="${SCOPE_DEPLOYMENT_COMPONENT:-}"
deployment_source_sha="${SCOPE_DEPLOYMENT_SOURCE_SHA:-${GITHUB_SHA:-}}"
deployment_evidence_path="${SCOPE_DEPLOYMENT_EVIDENCE_PATH:-}"
verified_successful_sha="${SCOPE_VERIFIED_SUCCESSFUL_SHA:-}"
defer_service_health="${SCOPE_DEFER_SERVICE_HEALTH:-0}"
deployment_was_skipped=0
prepared_release="${SCOPE_PREPARED_RELEASE_PATH:-}"
predecessor_teardown_dir="${SCOPE_PREDECESSOR_TEARDOWN_DIR:-}"
previous_deployment_ids="[]"
expected_config=""

if [[ "$defer_service_health" != "0" && "$defer_service_health" != "1" ]]; then
  echo "SCOPE_DEFER_SERVICE_HEALTH must be 0 or 1." >&2
  exit 2
fi

deploy_message_from_event() {
  local raw_message="${RAILWAY_DEPLOY_MESSAGE:-}"
  local first_line
  local pr_title

  first_line="$(printf '%s\n' "$raw_message" | sed -n '1p')"
  pr_title="$(printf '%s\n' "$raw_message" | awk 'NR > 1 && NF { print; exit }')"

  if [[ "$first_line" =~ ^Merge\ pull\ request\ #[0-9]+ ]] && [ -n "$pr_title" ]; then
    printf '%s\n' "$pr_title"
  elif [ -n "$first_line" ]; then
    printf '%s\n' "$first_line"
  else
    printf '%s\n' "${GITHUB_WORKFLOW:-Railway deploy}"
  fi
}

railway_read() {
  node "$(dirname "${BASH_SOURCE[0]}")/railway-read.mjs" "$@"
}

source "$(dirname "${BASH_SOURCE[0]}")/railway-service-health.sh"

ensure_service_exists() {
  local service_name="$1"
  local services_json

  services_json="$(
    railway_read service list \
      --project "$RAILWAY_PROJECT_ID" \
      --environment "$railway_environment" \
      --json
  )"

  if ! SERVICES_JSON="$services_json" SERVICE_NAME="$service_name" node -e 'const services = JSON.parse(process.env.SERVICES_JSON || "[]"); const name = process.env.SERVICE_NAME || ""; process.exit(services.some((service) => service.name === name || service.id === name) ? 0 : 1);'; then
    echo "Railway service '${service_name}' was not found in environment '${railway_environment}'."
    echo "Create the service in Railway, configure its variables, then rerun this workflow."
    return 1
  fi
}

print_deployment_logs() {
  local service_name="$1"
  local deployment_id="$2"

  echo "::group::Railway build logs for ${service_name}/${deployment_id}"
  railway logs "$deployment_id" \
    --project "$RAILWAY_PROJECT_ID" \
    --service "$service_name" \
    --environment "$railway_environment" \
    --build \
    --lines 200 || true
  echo "::endgroup::"

  echo "::group::Railway deploy logs for ${service_name}/${deployment_id}"
  railway logs "$deployment_id" \
    --project "$RAILWAY_PROJECT_ID" \
    --service "$service_name" \
    --environment "$railway_environment" \
    --deployment \
    --lines 200 || true
  echo "::endgroup::"
}

upload_contains_source_revision() {
  local marker
  [[ -n "$deployment_source_sha" ]] || return 1

  while IFS= read -r marker; do
    if [[ "$(tr -d '[:space:]' < "$marker")" == "$deployment_source_sha" ]]; then
      return 0
    fi
  done < <(find "$upload_root" -type f -name .scope-deployment-sha -print)
  return 1
}

record_deployment_evidence() {
  local deployment_id="$1"
  [[ -n "$deployment_evidence_path" ]] || return 0
  if [[ -z "$deployment_component" || -z "$deployment_source_sha" ]]; then
    echo "SCOPE_DEPLOYMENT_COMPONENT and SCOPE_DEPLOYMENT_SOURCE_SHA are required when recording evidence." >&2
    return 1
  fi
  if [[ -n "$prepared_release" ]]; then
    node .github/scripts/railway-artifact.mjs validate "$prepared_release" "$deployment_source_sha" "$deployment_component" >/dev/null
  elif ! upload_contains_source_revision; then
    echo "Railway upload for ${deployment_component} does not contain source revision ${deployment_source_sha}." >&2
    return 1
  fi

  # The JavaScript template literal is evaluated by Node.
  # shellcheck disable=SC2016
  EVIDENCE_PATH="$deployment_evidence_path" \
    COMPONENT="$deployment_component" \
    SOURCE_SHA="$deployment_source_sha" \
    DEPLOYMENT_ID="$deployment_id" \
    node -e '
const { appendFileSync } = require("node:fs");
appendFileSync(process.env.EVIDENCE_PATH, `${JSON.stringify({
  component: process.env.COMPONENT,
  sourceSha: process.env.SOURCE_SHA,
  provider: "railway",
  evidenceId: process.env.DEPLOYMENT_ID,
})}\n`);
'
}

wait_for_deployment() {
  local service_name="$1"
  local deployment_id="$2"
  local deadline=$((SECONDS + 900))
  local deployment_json
  local deployment_line
  local deployment_status
  local skipped_reason

  while true; do
    if deployment_json="$(
      railway_read deployment list \
        --project "$RAILWAY_PROJECT_ID" \
        --service "$service_name" \
        --environment "$railway_environment" \
        --limit 10 \
        --json
    )"; then
      deployment_line="$(
        DEPLOYMENTS_JSON="$deployment_json" \
        DEPLOYMENT_ID="$deployment_id" \
        node -e 'const deployments = JSON.parse(process.env.DEPLOYMENTS_JSON || "[]"); const id = process.env.DEPLOYMENT_ID || ""; const deployment = deployments.find((candidate) => candidate.id === id); if (deployment) console.log([deployment.id, deployment.status, deployment.meta?.skippedReason || ""].join("\t"));'
      )"

      if [ -n "$deployment_line" ]; then
        IFS=$'\t' read -r deployment_id deployment_status skipped_reason <<< "$deployment_line"
        echo "Railway deployment $deployment_id is $deployment_status."

        case "$deployment_status" in
          SUCCESS)
            return 0
            ;;
          SKIPPED)
            if [[ -n "$prepared_release" ]]; then
              echo "Prepared activation $deployment_id was skipped; refusing to substitute another deployment." >&2
              return 1
            fi
            echo "Railway skipped deployment: ${skipped_reason:-no reason provided}."
            if [[ -n "$deployment_component" && -n "$deployment_source_sha" \
              && "$verified_successful_sha" == "$deployment_source_sha" ]]; then
              echo "The durable deployment ledger already records this exact source revision as successful."
              deployment_was_skipped=1
              return 0
            fi
            echo "Refusing to infer source revision from current service health." >&2
            return 1
            ;;
          FAILED|CRASHED|REMOVED)
            print_deployment_logs "$service_name" "$deployment_id"
            return 1
            ;;
        esac
      else
        echo "Waiting for Railway deployment $deployment_id to appear..."
      fi
    else
      echo "Waiting for Railway deployment status..."
    fi

    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "Timed out waiting for Railway deployment."
      return 1
    fi

    sleep 10
  done
}

deploy_message="$(deploy_message_from_event)"
deploy_output=""
deployment_id=""

ensure_service_exists "$service_name"

if [[ -n "$prepared_release" ]]; then
  [[ -n "$deployment_component" && -n "$deployment_source_sha" ]] || {
    echo 'Prepared activation requires a component and exact source revision.' >&2
    exit 2
  }
  node .github/scripts/railway-artifact.mjs validate "$prepared_release" "$deployment_source_sha" "$deployment_component" >/dev/null
  expected_service="$(jq -er --arg component "$deployment_component" '.components[$component].serviceId' "$prepared_release")"
  [[ "$expected_service" == "$service_name" ]] || {
    echo 'Prepared artifact service does not match the activation target.' >&2
    exit 2
  }
  if [[ "$(node .github/scripts/deployment-components.mjs describe "$deployment_component" | jq -r '.verifyTransitionConfig')" == true ]]; then
    expected_config="$(railway_config_path "$deployment_component")"
  fi
  previous_deployment_ids="$(railway_read status \
    --project "$RAILWAY_PROJECT_ID" --environment "$railway_environment" --json |
    jq -ce --arg environment "$railway_environment" --arg service "$service_name" '
      [.environments.edges[].node | select(.id == $environment or .name == $environment)]
      | if length == 1 then .[0] else error("Railway environment is missing or ambiguous") end
      | [.serviceInstances.edges[].node | select(.serviceId == $service or .serviceName == $service)]
      | if length == 1 then .[0] else error("Railway service is missing or ambiguous") end
      | .activeDeployments
      | if type != "array" then error("Railway service is missing active deployments")
        elif all(.[]; (.id | type == "string" and test("^[A-Za-z0-9-]+$"))) then map(.id) | unique
        else error("Railway active deployment has an invalid ID") end
    ')"
  if [[ -n "$predecessor_teardown_dir" ]]; then
    node .github/scripts/railway-predecessor-teardown.mjs record \
      "$predecessor_teardown_dir" "$deployment_component" "$service_name" "$previous_deployment_ids"
  fi
  deploy_output="$(node .github/scripts/railway-artifact.mjs activate \
    "$prepared_release" "$deployment_component" "$railway_environment")"
else
  [[ -n "$upload_root" ]] || {
    echo 'usage: deploy-railway.sh <service-name> <upload-root>' >&2
    exit 2
  }
  if deploy_output="$(
    railway up "$upload_root" \
      --path-as-root \
      --no-gitignore \
      --project "$RAILWAY_PROJECT_ID" \
      --service "$service_name" \
      --environment "$railway_environment" \
      --message "$deploy_message" \
      --detach \
      --json 2>&1
  )"; then
    :
  else
    upload_status=$?
    # Report only a provider status code. Upload errors may contain signed URLs.
    upload_http_status="$(printf '%s' "$deploy_output" | node -e '
      let body = "";
      process.stdin.on("data", chunk => body += chunk);
      process.stdin.on("end", () => {
        for (const line of body.split("\n")) {
          try {
            const value = JSON.parse(line);
            const status = value.statusCode ?? value.status_code ?? value.status ?? value.error?.status;
            if (/^[45][0-9]{2}$/.test(String(status))) { console.log(status); return; }
          } catch {}
        }
        const match = body.match(/(?:HTTP(?:\/[0-9.]+)?[ :]+|status(?: code)?[ :=]+)([45][0-9]{2})\b/i)
          ?? body.match(/\b([45][0-9]{2}) (?:Bad Gateway|Not Found|Unauthorized|Forbidden|Service Unavailable|Gateway Timeout)\b/);
        if (match) console.log(match[1]);
      });
    ')"
    echo "Railway source upload failed (exit $upload_status${upload_http_status:+; HTTP $upload_http_status}). No deployment receipt was returned; inspect Railway before retrying." >&2
    exit "$upload_status"
  fi
fi
printf '%s\n' "$deploy_output"
deployment_id="$(printf '%s\n' "$deploy_output" | jq -er 'select(.deploymentId | type == "string" and length > 0) | .deploymentId' | tail -1)"

wait_for_deployment "$service_name" "$deployment_id"
if [[ -n "$prepared_release" ]]; then
  deployed_metadata="$(mktemp)"
  railway_read deployment list --project "$RAILWAY_PROJECT_ID" --environment "$railway_environment" \
    --service "$service_name" --limit 100 --json > "$deployed_metadata"
  if ! node .github/scripts/railway-artifact.mjs verify "$prepared_release" \
    "$deployment_component" "$deployed_metadata" "$deployment_id"; then
    rm -f "$deployed_metadata"
    exit 1
  fi
  rm -f "$deployed_metadata"
  if [[ -n "$predecessor_teardown_dir" ]]; then
    node .github/scripts/railway-predecessor-teardown.mjs activated \
      "$predecessor_teardown_dir" "$deployment_component" "$deployment_id"
  fi
fi
if [[ "$defer_service_health" == "0" ]]; then
  if [[ "$deployment_was_skipped" == "1" ]]; then
    service_is_healthy "$service_name" "" "$expected_config"
  else
    wait_for_service_health "$service_name" "$deployment_id" "$expected_config"
  fi
fi
if [[ -n "${SCOPE_RELEASE_DEPLOYMENTS_FILE:-}" ]]; then
  # Concurrent activations share this file; serialize the read-modify-write.
  (
    flock -x 9
    jq --arg component "$deployment_component" --arg id "$deployment_id" \
      '.[$component] = $id' "$SCOPE_RELEASE_DEPLOYMENTS_FILE" > "$SCOPE_RELEASE_DEPLOYMENTS_FILE.tmp.$$"
    mv "$SCOPE_RELEASE_DEPLOYMENTS_FILE.tmp.$$" "$SCOPE_RELEASE_DEPLOYMENTS_FILE"
  ) 9>>"$SCOPE_RELEASE_DEPLOYMENTS_FILE.lock"
fi
if [[ "$deployment_was_skipped" == "0" ]]; then
  record_deployment_evidence "$deployment_id"
fi

if [[ -n "$prepared_release" && -z "$predecessor_teardown_dir" ]]; then
  deadline=$((SECONDS + 600))
  while IFS= read -r previous_deployment_id; do
    [[ "$previous_deployment_id" != "$deployment_id" ]] || continue
    while true; do
      previous_status="$(railway_read deployment list \
        --project "$RAILWAY_PROJECT_ID" --environment "$railway_environment" \
        --service "$service_name" --limit 100 --json |
        jq -r --arg id "$previous_deployment_id" '.[] | select(.id == $id) | .status')"
      [[ "$previous_status" != REMOVED ]] || break
      if ((SECONDS >= deadline)); then
        echo "Previous deployment $previous_deployment_id has not completed teardown ($previous_status)." >&2
        exit 1
      fi
      sleep 5
    done
    echo "Previous deployment $previous_deployment_id completed teardown."
  done < <(jq -r '.[]' <<< "$previous_deployment_ids")
fi
