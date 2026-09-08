#!/usr/bin/env bash
set -euo pipefail

action="${1:?usage: staging-railway-token.sh <create|delete>}"
project_id="${SCOPE_RAILWAY_PROJECT_ID:?SCOPE_RAILWAY_PROJECT_ID is required}"
environment_id="${SCOPE_RAILWAY_STAGING_ENVIRONMENT_ID:?SCOPE_RAILWAY_STAGING_ENVIRONMENT_ID is required}"
token_name="${SCOPE_RAILWAY_PROJECT_TOKEN_NAME:?SCOPE_RAILWAY_PROJECT_TOKEN_NAME is required}"
: "${RAILWAY_API_TOKEN:?RAILWAY_API_TOKEN is required}"
# shellcheck source=.github/scripts/railway-graphql.sh
source "$(dirname "${BASH_SOURCE[0]}")/railway-graphql.sh"

list_token_ids() {
  local response
  response="$(
    # GraphQL variables are intentionally literal here.
    # shellcheck disable=SC2016
    railway_graphql read \
      'query ProjectTokens($projectId: String!) { projectTokens(projectId: $projectId) { edges { node { id name } } } }' \
      "$(jq -cn --arg projectId "$project_id" '{projectId: $projectId}')"
  )" || return $?
  jq -er '.data.projectTokens.edges | type == "array"' <<< "$response" >/dev/null || return 1
  jq -r --arg name "$token_name" \
    '.data.projectTokens.edges[]?.node | select(.name == $name) | .id' \
    <<< "$response"
}

delete_tokens() {
  local token_ids id attempt
  for attempt in 1 2 3; do
    token_ids="$(list_token_ids)" || return $?
    [[ -n "$token_ids" ]] || return 0
    # Only this run's exact token name is eligible for reconciliation.
    while IFS= read -r id; do
      # A lost delete response is reconciled by the next list, never by its body.
      # shellcheck disable=SC2016
      railway_graphql once \
        'mutation ProjectTokenDelete($id: String!) { projectTokenDelete(id: $id) }' \
        "$(jq -cn --arg id "$id" '{id: $id}')" >/dev/null || true
    done <<< "$token_ids"
    sleep 2
  done
  token_ids="$(list_token_ids)" || return $?
  [[ -z "$token_ids" ]] || { echo "Railway project token still exists after deletion." >&2; return 1; }
}

case "$action" in
  create)
    : "${GITHUB_ENV:?GITHUB_ENV is required when creating the staging token}"
    token_ids="$(list_token_ids)" || exit 1
    if [[ -n "$token_ids" ]]; then
      echo "A Railway project token already uses this staging run name." >&2
      exit 1
    fi
    # Never repeat creation: an HTTP failure can still have created a token.
    # shellcheck disable=SC2016
    if ! create_response="$(railway_graphql once \
      'mutation ProjectTokenCreate($input: ProjectTokenCreateInput!) { projectTokenCreate(input: $input) }' \
      "$(jq -cn --arg projectId "$project_id" --arg environmentId "$environment_id" \
        --arg name "$token_name" '{input: {projectId: $projectId, environmentId: $environmentId, name: $name}}')")"; then
      delete_tokens || echo "Could not confirm cleanup of the staging run token." >&2
      exit 1
    fi
    if ! project_token="$(jq -er '.data.projectTokenCreate | strings | select(length > 0)' <<< "$create_response")"; then
      delete_tokens || echo "Could not confirm cleanup of the staging run token." >&2
      exit 1
    fi
    printf '::add-mask::%s\n' "$project_token"
    token_ids="$(list_token_ids)" || { delete_tokens; exit 1; }
    if [[ "$(wc -l <<< "$token_ids")" -ne 1 || -z "$token_ids" ]]; then
      echo "Railway did not return one project token for this staging run." >&2
      delete_tokens
      exit 1
    fi
    printf 'RAILWAY_TOKEN=%s\n' "$project_token" >> "$GITHUB_ENV"
    ;;
  delete)
    delete_tokens
    ;;
  *)
    echo "usage: staging-railway-token.sh <create|delete>" >&2
    exit 2
    ;;
esac
