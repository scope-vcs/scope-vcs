#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/railway-private-command.sh"
environment="${1:?Maintenance environment is required}"
command="${2:?Maintenance command is required}"
[[ "$#" == 2 ]]
case "$command" in
  preflight|plan|verify|fence|drain-writers|validate-workflow-catalogs|apply|backfill-workflow-catalogs) ;;
  *) echo 'Unsupported private maintenance command.' >&2; exit 2 ;;
esac
binary="${SCOPE_MAINTENANCE_BINARY:-./target/release/scope-maintenance}"
prepared="${SCOPE_PREPARED_RELEASE_PATH:?Prepared release manifest is required}"
node "$(dirname "${BASH_SOURCE[0]}")/railway-artifact.mjs" verify-maintenance "$prepared" "$binary" >/dev/null
digest="$(jq -er '.maintenanceSha256' "$prepared")"
manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
lock="$(jq -er '.releasePolicy.migrationLockTimeoutSeconds | select(type == "number" and . > 0 and floor == .)' "$manifest")"
statement="$(jq -er '.releasePolicy.migrationStatementTimeoutSeconds | select(type == "number" and . > 0 and floor == .)' "$manifest")"
railway_private_command "$environment" sh -ceu '
  umask 077
  directory=$(mktemp -d /tmp/scope-maintenance.XXXXXXXX)
  trap '\''rm -rf -- "$directory"'\'' EXIT
  trap "exit 1" HUP INT TERM
  cat > "$directory/scope-maintenance"
  printf "%s  %s\n" "$1" "$directory/scope-maintenance" | sha256sum --check --status
  chmod 0700 "$directory/scope-maintenance"
  export SCOPE_DATA_DIR="$directory/data"
  export SCOPE_MIGRATION_LOCK_TIMEOUT_SECONDS="$3"
  export SCOPE_MIGRATION_STATEMENT_TIMEOUT_SECONDS="$4"
  "$directory/scope-maintenance" "$2"
' scope-maintenance "$digest" "$command" "$lock" "$statement" < "$binary"
# New tables default to no runtime access. Apply the reviewed service grants
# before the cutover can reopen writers, including after a staging schema restore.
if [[ "$command" == apply ]]; then
  node deploy/postgres/runtime-roles.mjs --grants-only | \
    railway_private_command "$environment" sh -ceu 'exec psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1' >/dev/null
fi
