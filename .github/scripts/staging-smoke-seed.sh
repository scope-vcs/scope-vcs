#!/usr/bin/env bash
set -euo pipefail
umask 077

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
source "$scripts/railway-private-command.sh"
scope=(--project "$project_id" --environment "$environment_id")

SCOPE_DEPLOYMENT_MANIFEST_JSON="$(jq -c . "$manifest")" \
  SCOPE_RAILWAY_STATUS_JSON="$(node "$scripts/railway-read.mjs" status "${scope[@]}" --json)" \
  SCOPE_RAILWAY_SERVICES_JSON="$(node "$scripts/railway-read.mjs" service list "${scope[@]}" --json)" \
  node "$scripts/verify-staging-target.mjs" >/dev/null

# Upload the candidate tool over SSH and mint the grant inside Railway's private network.
# The remote helper validates Railway's injected identity before touching metadata.
seed_sha="$(sha256sum "$seed_binary" | cut -d ' ' -f1)"
remote='
set -eu
umask 077
directory=$(mktemp -d)
trap "rm -rf \"\$directory\"" EXIT
trap "exit 1" HUP INT TERM
cat > "$directory/seed"
printf "%s  %s\n" "$1" "$directory/seed" | sha256sum --check --status
chmod 0700 "$directory/seed"
export SCOPE_ALLOW_STAGING_SMOKE_SEED=1
export SCOPE_SMOKE_SEED_PROJECT_ID="$2"
export SCOPE_SMOKE_SEED_ENVIRONMENT_ID="$3"
export SCOPE_SMOKE_SEED_ENVIRONMENT_NAME="$4"
export SCOPE_PRODUCTION_ENVIRONMENT_ID="$5"
export SCOPE_SMOKE_SEED_USER_EMAIL=smoke@example.test
export SCOPE_SMOKE_SEED_USER_HANDLE=dev
export SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH="$directory/exchange-token"
"$directory/seed" --grant-only >/dev/null
cat "$SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH"
'
# Never overwrite an existing credential, and remove partial output on SSH failure.
set -o noclobber
exec 3> "$SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH"
if ! railway_private_command "$environment_id" sh -c "$remote" scope-smoke-seed \
  "$seed_sha" "$project_id" "$environment_id" "$environment_name" "$production_id" \
  < "$seed_binary" >&3; then
  rm -f "$SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH"
  exit 1
fi
exec 3>&-
test -s "$SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH"
