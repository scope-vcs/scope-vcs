#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/railway-private-command.sh"
name="${1:?staging or production is required}"
receipt="${2:?Runtime receipt is required}"
[[ "$name" == staging || "$name" == production ]]
manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
environment="$(jq -er --arg name "$name" '.environments[$name].environmentId' "$manifest")"
digest="$(jq -er '.maintenanceSha256 | select(test("^[a-f0-9]{64}$"))' "$receipt")"
railway_private_command "$environment" sh -ceu '
  printf "%s  /app/bin/scope-maintenance\n" "$1" | sha256sum --check --status
  # Railway SSH runs an administrator shell; inspect the running service instead.
  test "$(awk "/^Uid:/ {print \$3}" /proc/1/status)" = 65532
  case "$DATABASE_URL" in
    *".railway.internal:"*|*".railway.internal/"*) ;;
    *) echo "Maintenance requires private Railway database networking" >&2; exit 2;;
  esac
  /app/bin/scope-maintenance preflight
' scope-maintenance-verify "$digest" < /dev/null
