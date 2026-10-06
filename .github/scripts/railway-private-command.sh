#!/usr/bin/env bash
railway_private_command() (
  local environment="$1"; shift
  local manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
  local project service production staging preview remote command argument
  project="$(jq -er '.railway.projectId' "$manifest")" || return
  service="${SCOPE_RAILWAY_MAINTENANCE_SERVICE_ID:-$(jq -er '.railway.maintenanceServiceId' "$manifest")}" || return
  production="$(jq -er '.environments.production.environmentId' "$manifest")" || return
  staging="$(jq -er '.environments.staging.environmentId' "$manifest")" || return
  preview="${SCOPE_RAILWAY_PREVIEW_ENVIRONMENT_ID:-}"
  [[ -z "$preview" || ( "$preview" != "$production" && "$preview" != "$staging" ) ]] || { echo 'Preview environment must differ from production and staging.' >&2; return 2; }
  [[ "$environment" == "$production" || "$environment" == "$staging" || ( -n "$preview" && "$environment" == "$preview" ) ]] || { echo 'Unknown maintenance environment.' >&2; return 2; }
  for argument in "$project" "$service" "$environment"; do
    [[ "$argument" =~ ^[a-fA-F0-9]{8}(-[a-fA-F0-9]{4}){3}-[a-fA-F0-9]{12}$ ]] || { echo 'Maintenance requires explicit UUID targets.' >&2; return 2; }
  done
  command=''
  for argument in "$@"; do
    argument="${argument//\'/\'\\\'\'}"
    command+=" '$argument'"
  done
  remote="test \"\$RAILWAY_PROJECT_ID\" = '$project' && test \"\$RAILWAY_ENVIRONMENT_ID\" = '$environment' && test \"\$RAILWAY_SERVICE_ID\" = '$service' || exit 2; exec$command"
  local identity=()
  if [[ -n "${SCOPE_RAILWAY_SSH_PRIVATE_KEY:-}" ]]; then
    scope_private_key_file="$(mktemp "${RUNNER_TEMP:-/tmp}/scope-railway-key.XXXXXXXX")" || return
    trap 'rm -f -- "$scope_private_key_file"' EXIT
    chmod 0600 "$scope_private_key_file" || return
    printf '%s\n' "$SCOPE_RAILWAY_SSH_PRIVATE_KEY" >| "$scope_private_key_file" || return
    unset SCOPE_RAILWAY_SSH_PRIVATE_KEY
    SCOPE_RAILWAY_SSH_IDENTITY_FILE="$scope_private_key_file"
  fi
  [[ -z "${SCOPE_RAILWAY_SSH_IDENTITY_FILE:-}" ]] || identity=(--identity-file "$SCOPE_RAILWAY_SSH_IDENTITY_FILE")
  local ssh_bin
  ssh_bin="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../deploy/railway/ssh-bin" && pwd)" || return
  PATH="$ssh_bin:$PATH" railway ssh --project "$project" --environment "$environment" --service "$service" "${identity[@]}" -- "$remote"
)

railway_private_read() (
  local attempt status
  scope_read_input="$(mktemp "${RUNNER_TEMP:-/tmp}/scope-railway-read.XXXXXXXX")" || return
  scope_read_output="$(mktemp "${RUNNER_TEMP:-/tmp}/scope-railway-read.XXXXXXXX")" || return
  trap 'rm -f -- "$scope_read_input" "$scope_read_output"' EXIT
  cat >| "$scope_read_input" || return
  for attempt in 1 2 3; do
    status=0
    railway_private_command "$@" < "$scope_read_input" >| "$scope_read_output" || status=$?
    if [[ "$status" != 255 || "$attempt" == 3 ]]; then
      cat -- "$scope_read_output" || return
      return "$status"
    fi
    echo "Railway SSH transport failed; retrying read-only command ($attempt/3)." >&2
    sleep $((attempt * 2))
  done
)
