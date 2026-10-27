#!/usr/bin/env bash
set -euo pipefail

: "${SCOPE_BINARY:?Set SCOPE_BINARY to the built scope executable}"
: "${SCOPE_ARTIFACT:?Set SCOPE_ARTIFACT to the output archive path}"

workspace="$(mktemp -d)"
trap 'rm -rf "$workspace"' EXIT

bundle="$workspace/bundle"
licenses="$bundle/licenses"
mkdir -p "$licenses" "$(dirname "$SCOPE_ARTIFACT")"

scope_name=scope
[[ "$SCOPE_BINARY" == *.exe ]] && scope_name=scope.exe
cp "$SCOPE_BINARY" "$bundle/$scope_name"
chmod 0755 "$bundle/$scope_name"
cp LICENSE NOTICE legal/third-party-cli.txt "$licenses/"

tar -czf "$SCOPE_ARTIFACT" -C "$bundle" .

artifact_bytes="$(wc -c < "$SCOPE_ARTIFACT" | tr -d '[:space:]')"
max_artifact_bytes="$(jq -er '.max_artifact_bytes' cli/distribution/targets.json)"
if (( artifact_bytes > max_artifact_bytes )); then
  echo "$SCOPE_ARTIFACT is $artifact_bytes bytes, over the $max_artifact_bytes byte cap by $((artifact_bytes - max_artifact_bytes)) bytes" >&2
  exit 1
fi
awk -v name="$(basename "$SCOPE_ARTIFACT")" -v bytes="$artifact_bytes" \
  'BEGIN { printf "%s is %.1f MiB\n", name, bytes / 1048576 }'
