#!/usr/bin/env bash
# Retry only reads. Mutations require operation-specific reconciliation.
railway_graphql() {
  local mode="$1" query="$2" variables="$3"
  local request response attempt attempts=1 status
  [[ "$mode" == read ]] && attempts=3
  request="$(jq -cn --arg query "$query" --argjson variables "$variables" '{query: $query, variables: $variables}')" || return $?
  for ((attempt=1; attempt<=attempts; attempt++)); do
    status=0
    response="$(curl --silent --show-error --fail-with-body \
      --connect-timeout 10 --max-time 30 \
      --request POST --url https://backboard.railway.com/graphql/v2 \
      --header "Authorization: Bearer $RAILWAY_API_TOKEN" \
      --header 'Content-Type: application/json' --data-binary "$request" 2>/dev/null)" || status=$?
    if [[ "$status" == 0 ]] && jq -e 'type == "object" and (.errors // [] | length == 0) and (.data | type == "object")' <<< "$response" >/dev/null 2>&1; then
      printf '%s\n' "$response"
      return 0
    fi
    # Never emit provider response bodies: token and registry requests contain secrets.
    if ((attempt < attempts)); then
      echo "Railway read failed; retrying ($attempt/$attempts)." >&2
      sleep 2
    fi
  done
  echo "Railway GraphQL request failed after $attempts attempt(s)." >&2
  return 1
}
