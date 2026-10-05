#!/usr/bin/env bash
set -euo pipefail

manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
project="$(jq -er '.railway.projectId' "$manifest")"
environment="$(jq -er '.environments.production.environmentId' "$manifest")"

deployments="$(env -u GITHUB_OUTPUT node .github/scripts/production-deployment-progress.mjs read | jq -ec .deployments)"
services="$(node .github/scripts/railway-read.mjs status \
  --project "$project" --environment "$environment" --json)"
if [[ -n "${SCOPE_REPLACE_FAILED_WEB_RUN_ID:-}" ]]; then
  replacement="$(SCOPE_DEPLOYMENT_MANIFEST_JSON="$(jq -c . "$manifest")" \
    SCOPE_RAILWAY_SERVICES_JSON="$services" \
    node .github/scripts/failed-web-replacement.mjs)"
  deployments="$(jq -ec --argjson web "$replacement" '.web = $web' <<< "$deployments")"
fi
if [[ -n "${SCOPE_REPLACE_REDEPLOYED_COMPONENT:-}" ]]; then
  replacement="$(SCOPE_DEPLOYMENT_MANIFEST_JSON="$(jq -c . "$manifest")" \
    SCOPE_PRODUCTION_DEPLOYMENTS_JSON="$deployments" \
    SCOPE_RAILWAY_SERVICES_JSON="$services" \
    node .github/scripts/redeployed-service-replacement.mjs)"
  deployments="$(jq -ec --argjson replacement "$replacement" '. + $replacement' <<< "$deployments")"
fi
source "$(dirname "${BASH_SOURCE[0]}")/railway-private-command.sh"
SCOPE_DEPLOYMENT_MANIFEST_JSON="$(jq -c . "$manifest")" \
  SCOPE_PRODUCTION_DEPLOYMENTS_JSON="$deployments" \
  SCOPE_RAILWAY_SERVICES_JSON="$services" \
  node .github/scripts/production-readiness-preflight.mjs | \
  railway_private_read "$environment" sh -ceu \
    'exec psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1' >/dev/null

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  printf 'deployments=%s\n' "$deployments" >> "$GITHUB_OUTPUT"
fi
echo 'Production deployment receipts, live Railway services, and database roles are ready.'
