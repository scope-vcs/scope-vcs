#!/usr/bin/env bash
set -euo pipefail

manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
project="$(jq -er '.railway.projectId' "$manifest")"
environment="$(jq -er '.environments.production.environmentId' "$manifest")"

# This owner writes only step outputs when GITHUB_OUTPUT is present. Here its
# JSON is an input to health verification, not an Actions output.
deployments="$(env -u GITHUB_OUTPUT node .github/scripts/production-deployment-progress.mjs read | jq -ec .deployments)"
services="$(node .github/scripts/railway-read.mjs status \
  --project "$project" --environment "$environment" --json)"
source "$(dirname "${BASH_SOURCE[0]}")/railway-private-command.sh"
SCOPE_DEPLOYMENT_MANIFEST_JSON="$(jq -c . "$manifest")" \
  SCOPE_PRODUCTION_DEPLOYMENTS_JSON="$deployments" \
  SCOPE_RAILWAY_SERVICES_JSON="$services" \
  node .github/scripts/production-readiness-preflight.mjs | \
  railway_private_command "$environment" sh -ceu \
    'exec psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1' >/dev/null

echo 'Production deployment receipts, live Railway services, and database roles are ready.'
