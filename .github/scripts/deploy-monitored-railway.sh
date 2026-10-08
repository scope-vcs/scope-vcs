#!/usr/bin/env bash
set -euo pipefail
stage="${1:?usage: deploy-monitored-railway.sh backend|web COMMAND [ARGS...]}"
shift
node .github/scripts/railway-tracing.mjs production
output="$(realpath -m "release-evidence/$stage")"
mkdir -p "$output"
export SCOPE_RELEASE_DEPLOYMENTS_FILE="$output/deployments.json"
export SCOPE_RELEASE_MAINTENANCE_START_FILE="$output/maintenance-start"
export SCOPE_RELEASE_MAINTENANCE_END_FILE="$output/maintenance-end"
env -u RAILWAY_API_TOKEN railway service list --project "$RAILWAY_PROJECT_ID" --environment "$SCOPE_RAILWAY_ENVIRONMENT_ID" --json > "$output/services.json"
jq --slurpfile manifest .github/deployment-services.json '
  . as $services | $manifest[0].services | to_entries |
  map(. as $component | $services[] | select(.id == $component.value.id) |
    select(.deploymentId != null) | {key: $component.key, value: .deploymentId}) | from_entries
' "$output/services.json" > "$SCOPE_RELEASE_DEPLOYMENTS_FILE"
rm "$output/services.json"
SCOPE_RELEASE_OBSERVATION_SECONDS="$(jq -er '.releasePolicy.postReleaseObservationSeconds | select(type == "number" and . > 0)' .github/deployment-services.json)"
export SCOPE_RELEASE_OBSERVATION_SECONDS
node .github/scripts/production-availability-config.mjs "$stage" "$output"
if [[ "${SCOPE_RECOVER_CLOSED_CUTOVER:-0}" == 1 ]]; then
  node .github/scripts/production-deployment-progress.mjs cutover-read \
    --id "$SCOPE_RELEASE_CUTOVER_ID" --source-sha "$SCOPE_DEPLOYMENT_SOURCE_SHA" > "$output/cutover.json"
  node --input-type=module - "$output/cutover.json" "$SCOPE_RELEASE_MAINTENANCE_START_FILE" <<'NODE'
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
const journal = JSON.parse(readFileSync(process.argv[2], "utf8"));
const closures = journal.events.filter(({phase}) => phase === "closing" || phase === "reclosing");
const start = closures.length ? Math.min(...closures.map(({at}) => Date.parse(at))) : Date.now();
if (!Number.isFinite(start)) throw new Error("Recovery closure timestamp is invalid");
execFileSync("bash", [".github/scripts/write-release-marker.sh", process.argv[3], String(start)]);
NODE
fi
bash .github/scripts/with-release-availability.sh "$output/config.json" "$output" -- "$@"
