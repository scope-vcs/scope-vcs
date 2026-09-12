#!/usr/bin/env bash
set -euo pipefail

: "${SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH:?SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH is required}"
seed_binary="${SCOPE_SMOKE_SEED_BINARY:-./target/release/scope-smoke-seed}"
test -x "$seed_binary"
if [[ -z "${RAILWAY_TOKEN:-}" || -n "${RAILWAY_API_TOKEN:-}" ]]; then
  echo 'Smoke credentials require only a staging-scoped Railway token.' >&2
  exit 2
fi

manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
scripts="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_id="$(jq -er '.railway.projectId' "$manifest")"
environment_id="$(jq -er '.environments.staging.environmentId' "$manifest")"
environment_name="$(jq -er '.environments.staging.environmentName' "$manifest")"
production_id="$(jq -er '.environments.production.environmentId' "$manifest")"
api_service="$(jq -er '.services.api.id' "$manifest")"
database_service="$(jq -er '.railway.databaseServiceId' "$manifest")"
scope=(--project "$project_id" --environment "$environment_id")

SCOPE_DEPLOYMENT_MANIFEST_JSON="$(jq -c . "$manifest")" \
  SCOPE_RAILWAY_STATUS_JSON="$(node "$scripts/railway-read.mjs" status "${scope[@]}" --json)" \
  SCOPE_RAILWAY_SERVICES_JSON="$(node "$scripts/railway-read.mjs" service list "${scope[@]}" --json)" \
  node "$scripts/verify-staging-target.mjs" >/dev/null

database_variables="$(node "$scripts/railway-read.mjs" variable list "${scope[@]}" --service "$database_service" --json)"
export SCOPE_STAGING_DATABASE_PUBLIC_URL
SCOPE_STAGING_DATABASE_PUBLIC_URL="$(jq -er '.DATABASE_PUBLIC_URL | strings | select(length > 0)' <<< "$database_variables")"
export SCOPE_ALLOW_STAGING_SMOKE_SEED=1
export SCOPE_SMOKE_SEED_PROJECT_ID="$project_id"
export SCOPE_SMOKE_SEED_ENVIRONMENT_ID="$environment_id"
export SCOPE_SMOKE_SEED_ENVIRONMENT_NAME="$environment_name"
export SCOPE_PRODUCTION_ENVIRONMENT_ID="$production_id"
export SCOPE_SMOKE_SEED_USER_EMAIL=smoke@example.test
export SCOPE_SMOKE_SEED_USER_HANDLE=dev
# Railway injects the target's identity and variables into the runner-local command.
# The Rust helper validates that identity before resetting data or minting a grant.
# shellcheck disable=SC2016
railway run "${scope[@]}" --service "$api_service" --no-local -- \
  sh -c 'DATABASE_URL="$SCOPE_STAGING_DATABASE_PUBLIC_URL" exec "$@"' \
  scope-smoke-seed "$seed_binary" --grant-only
