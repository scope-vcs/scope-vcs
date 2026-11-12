#!/usr/bin/env bash
set -euo pipefail
shopt -s inherit_errexit
umask 077
mode="${1:-reconcile}"
case "$mode:$#" in
  reconcile:0|reconcile:1)
    : "${SCOPE_PRODUCTION_MIGRATION_PLAN:?Production migration plan is required}" ;;
  provision:2)
    artifact_id="$2"
    [[ "$artifact_id" =~ ^[1-9][0-9]*$ ]] || { echo 'Baseline artifact ID must be a positive integer.' >&2; exit 2; }
    : "${SCOPE_STAGING_BASELINE_KEY:?Staging baseline encryption key is required to restore a snapshot}" ;;
  *) echo 'Usage: staging-baseline.sh [reconcile | provision ARTIFACT_ID]' >&2; exit 2 ;;
esac
: "${SCOPE_STAGING_BASELINE_DIR:?Staging baseline directory is required}"
: "${SCOPE_MAINTENANCE_BINARY:?Pinned maintenance binary is required}"
[[ -n "${RAILWAY_TOKEN:-}" && -z "${RAILWAY_API_TOKEN:-}" ]]
manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
project="$(jq -er '.railway.projectId' "$manifest")"
environment="$(jq -er '.environments.staging.environmentId' "$manifest")"
scripts="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$scripts/railway-private-command.sh"
source "$scripts/staging-deletion-window.sh"
maintenance() { bash "$scripts/railway-private-maintenance.sh" "$environment" "$@"; }
staging_sql() {
  railway_private_read "$environment" sh -c 'exec psql "$DATABASE_URL" -XAt -v ON_ERROR_STOP=1 -c "$1"' scope-baseline "$1" < /dev/null
}
release_baseline_run() {
  local run_id
  : "${GITHUB_REPOSITORY:?}"
  run_id="$(gh api "repos/$GITHUB_REPOSITORY/actions/artifacts/$1" \
    --jq 'select(.expired == false and .workflow_run.head_branch == "main") | .workflow_run.id')"
  [[ "$run_id" =~ ^[1-9][0-9]*$ ]] || { echo 'Retained staging baseline is expired or not from main.' >&2; return 1; }
  gh api "repos/$GITHUB_REPOSITORY/actions/runs/$run_id" \
    --jq '.path == ".github/workflows/release.yml" and .head_branch == "main" and (.event == "schedule" or .event == "workflow_dispatch")' | grep -qx true
  printf '%s\n' "$run_id"
}

if [[ "$mode" == provision ]]; then
  : "${GITHUB_REPOSITORY:?}"
  key="$(gh api "repos/$GITHUB_REPOSITORY/actions/artifacts/$artifact_id" --jq '.name | capture("^staging-baseline-(?<key>[0-9a-f]{64})$").key')"
  [[ "$key" =~ ^[0-9a-f]{64}$ ]] || { echo 'Artifact is not a retained staging baseline.' >&2; exit 1; }
  run_id="$(release_baseline_run "$artifact_id")"
  jq -e --arg run "$run_id" '.preparationRunId == $run' "${SCOPE_PREPARED_RELEASE_PATH:?Prepared release manifest is required}" >/dev/null || {
    echo "The maintenance binary must come from baseline run $run_id." >&2
    exit 1
  }
fi
[[ "$environment" != "$(jq -er '.environments.production.environmentId' "$manifest")" ]]
scope=(--project "$project" --environment "$environment")
SCOPE_DEPLOYMENT_MANIFEST_JSON="$(cat "$manifest")" \
  SCOPE_RAILWAY_STATUS_JSON="$(node deploy/scripts/railway-read.mjs status "${scope[@]}" --json)" \
  SCOPE_RAILWAY_SERVICES_JSON="$(node deploy/scripts/railway-read.mjs service list "${scope[@]}" --json)" \
  node deploy/scripts/verify-staging-target.mjs >/dev/null
maintenance fence >/dev/null
mkdir -p "$SCOPE_STAGING_BASELINE_DIR"
chmod 0700 "$SCOPE_STAGING_BASELINE_DIR"
cleanup() {
  rm -f "$SCOPE_STAGING_BASELINE_DIR/database.dump"
  rm -rf "$SCOPE_STAGING_BASELINE_DIR/restore" "$SCOPE_STAGING_BASELINE_DIR/restore.zip"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

download_snapshot() {
  local artifact="$1" key="$2"
  gh api "repos/$GITHUB_REPOSITORY/actions/artifacts/$artifact/zip" > "$SCOPE_STAGING_BASELINE_DIR/restore.zip"
  unzip -q "$SCOPE_STAGING_BASELINE_DIR/restore.zip" -d "$SCOPE_STAGING_BASELINE_DIR/restore"
  jq -e --arg environment "$environment" --arg key "$key" \
    '.environmentId == $environment and .ledgerHash == $key and (.metadataRestoreSafe | type) == "boolean"' \
    "$SCOPE_STAGING_BASELINE_DIR/restore/baseline.json" >/dev/null
  (cd "$SCOPE_STAGING_BASELINE_DIR/restore" &&
    [[ "$(wc -l < database.sha256)" == 1 ]] &&
    grep -Eq '^[0-9a-f]{64}  database\.dump\.enc$' database.sha256 &&
    sha256sum --check database.sha256 >/dev/null)
}

restore_snapshot() {
  local since="${1:-}"
  node deploy/scripts/staging-baseline-crypto.mjs decrypt \
    "$SCOPE_STAGING_BASELINE_DIR/restore/database.dump.enc" \
    "$SCOPE_STAGING_BASELINE_DIR/restore/database.dump" \
    "$SCOPE_STAGING_BASELINE_DIR/restore/baseline.json"
  pg_restore --no-owner --no-privileges --exit-on-error \
    --file="$SCOPE_STAGING_BASELINE_DIR/restore/database.sql" "$SCOPE_STAGING_BASELINE_DIR/restore/database.dump"
  {
    [[ -z "$since" ]] || record_deletions_since "$since"
    printf '%s\n' 'DROP SCHEMA public CASCADE; CREATE SCHEMA public;'
    cat "$SCOPE_STAGING_BASELINE_DIR/restore/database.sql"
    [[ -z "$since" ]] || require_snapshot_expected_deletions
  } | railway_private_command "$environment" sh -c 'exec psql "$DATABASE_URL" -X --single-transaction -v ON_ERROR_STOP=1' >/dev/null
}

record_deletions_since() {
  cat <<SQL
CREATE TEMP TABLE scope_deleted_sources ON COMMIT DROP AS
  SELECT object_key::text AS object_key FROM public.scope_orphan_object_jobs
  WHERE updated_at_unix >= $1 AND (attempts > 0 OR completed_at_unix IS NOT NULL
    OR next_run_at_unix - updated_at_unix NOT IN ($source_blob_queue_delay_seconds, $staging_deletion_window_seconds));
CREATE TEMP TABLE scope_deleted_repository_storage ON COMMIT DROP AS
  SELECT repo_id::text AS repo_id, generation::text AS generation, incarnation_id::text AS incarnation_id
  FROM public.scope_repo_storage_cleanup_jobs
  WHERE updated_at_unix >= $1 AND (attempts > 0 OR completed_at_unix IS NOT NULL
    OR next_run_at_unix - updated_at_unix NOT IN (0, $staging_deletion_window_seconds));
CREATE TEMP TABLE scope_deleted_media ON COMMIT DROP AS
  SELECT attachment_id::text AS attachment_id FROM public.scope_request_media_cleanup_jobs
  WHERE updated_at_unix >= $1 AND (attempt > 0 OR completed_at_unix IS NOT NULL);
SQL
}

require_snapshot_expected_deletions() {
  cat <<'SQL'
DO $guard$ BEGIN
  IF EXISTS (SELECT 1 FROM pg_temp.scope_deleted_sources deleted
      JOIN public.scope_object_references reference ON reference.object_key = deleted.object_key)
    OR EXISTS (SELECT 1 FROM pg_temp.scope_deleted_repository_storage deleted
      JOIN public.scope_repositories repository ON repository.id = deleted.repo_id
        AND (deleted.incarnation_id IS NULL OR repository.incarnation_id = deleted.incarnation_id)
      WHERE NOT EXISTS (SELECT 1 FROM public.scope_repo_storage_cleanup_jobs job
        WHERE job.repo_id = deleted.repo_id AND job.generation = deleted.generation AND job.completed_at_unix IS NULL))
    OR EXISTS (SELECT 1 FROM pg_temp.scope_deleted_media deleted
      JOIN public.scope_request_media_attachments attachment ON attachment.id = deleted.attachment_id
      WHERE NOT EXISTS (SELECT 1 FROM public.scope_request_media_cleanup_jobs job
        WHERE job.attachment_id = deleted.attachment_id AND job.completed_at_unix IS NULL)) THEN
    RAISE EXCEPTION 'Objects live in this snapshot were deleted after it was taken';
  END IF;
END $guard$;
SQL
}

require_seeded_staging() {
  maintenance preflight >/dev/null
  [[ "$(staging_sql 'SELECT count(*) > 0 FROM scope_repositories')" == t ]]
}

if [[ "$mode" == provision ]]; then
  download_snapshot "$artifact_id" "$key"
  since="$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$run_id" --jq '.created_at | fromdateiso8601')"
  [[ "$since" =~ ^[1-9][0-9]*$ ]]
  restore_snapshot "$since"
  maintenance plan > "$SCOPE_STAGING_BASELINE_DIR/current-plan.json"
  [[ "$(node deploy/scripts/staging-baseline.mjs "$SCOPE_STAGING_BASELINE_DIR/current-plan.json")" == "$key" ]]
  require_seeded_staging
  echo "Restored staging baseline artifact $artifact_id from release run $run_id; writers stay fenced until the next release migrates staging."
  exit 0
fi

key="$(node deploy/scripts/staging-baseline.mjs "$SCOPE_PRODUCTION_MIGRATION_PLAN")"
if ! (maintenance plan > "$SCOPE_STAGING_BASELINE_DIR/current-plan.json" &&
  node deploy/scripts/staging-baseline.mjs "$SCOPE_PRODUCTION_MIGRATION_PLAN" "$SCOPE_STAGING_BASELINE_DIR/current-plan.json" >/dev/null); then
  : "${SCOPE_STAGING_BASELINE_KEY:?Staging baseline encryption key is required to restore a snapshot}"
  : "${GITHUB_REPOSITORY:?}"
  artifact_id="$(gh api --paginate "repos/$GITHUB_REPOSITORY/actions/artifacts?name=staging-baseline-$key&per_page=100" \
    --jq '.artifacts[] | select(.expired == false and .workflow_run.head_branch == "main") | .id' | sed -n '1p')"
  [[ -n "$artifact_id" ]] || { echo 'No retained staging baseline matches production; explicit baseline provisioning is required.' >&2; exit 1; }
  release_baseline_run "$artifact_id" >/dev/null
  download_snapshot "$artifact_id" "$key"
  jq -e '.metadataRestoreSafe' "$SCOPE_STAGING_BASELINE_DIR/restore/baseline.json" >/dev/null || {
    echo "Retained staging baseline $artifact_id is not metadata-restore-safe; restore it with the Provision staging baseline workflow." >&2
    exit 1
  }
  restore_snapshot
  maintenance plan > "$SCOPE_STAGING_BASELINE_DIR/current-plan.json"
  node deploy/scripts/staging-baseline.mjs "$SCOPE_PRODUCTION_MIGRATION_PLAN" "$SCOPE_STAGING_BASELINE_DIR/current-plan.json" >/dev/null
fi
require_seeded_staging
if jq -e '.pending | length == 0' "$SCOPE_PRODUCTION_MIGRATION_PLAN" >/dev/null; then
  exit 0
fi
: "${SCOPE_STAGING_BASELINE_KEY:?Staging baseline encryption key is required to retain a snapshot}"
restore_safe="$(jq -r 'if (.metadataRestoreSafe | type) == "boolean" then .metadataRestoreSafe else error("Migration plan must declare metadataRestoreSafe") end' "$SCOPE_PRODUCTION_MIGRATION_PLAN")"
railway_private_command "$environment" sh -c 'exec pg_dump --dbname="$DATABASE_URL" --format=custom --no-owner --no-privileges' > "$SCOPE_STAGING_BASELINE_DIR/database.dump"
jq -n --argjson safe "$restore_safe" --arg environment "$environment" --arg key "$key" \
  '{environmentId: $environment, ledgerHash: $key, metadataRestoreSafe: $safe}' > "$SCOPE_STAGING_BASELINE_DIR/baseline.json"
node deploy/scripts/staging-baseline-crypto.mjs encrypt \
  "$SCOPE_STAGING_BASELINE_DIR/database.dump" \
  "$SCOPE_STAGING_BASELINE_DIR/database.dump.enc" \
  "$SCOPE_STAGING_BASELINE_DIR/baseline.json"
(cd "$SCOPE_STAGING_BASELINE_DIR" && sha256sum database.dump.enc > database.sha256)
echo "artifact_name=staging-baseline-$key" >> "$GITHUB_OUTPUT"
