#!/usr/bin/env bash
set -euo pipefail

: "${SCOPE_BINARY:?Set SCOPE_BINARY to the built scope executable}"
: "${SCOPE_ARTIFACT:?Set SCOPE_ARTIFACT to the output archive path}"
: "${SCOPE_NODE_VERSION:?Set SCOPE_NODE_VERSION to the pinned Node version}"
: "${SCOPE_NODE_PLATFORM:?Set SCOPE_NODE_PLATFORM to the Node distribution platform}"
: "${SCOPE_NODE_SHA256:?Set SCOPE_NODE_SHA256 to the pinned Node archive checksum}"

case "$SCOPE_NODE_PLATFORM" in
  win-*) node_extension=zip; node_executable=node.exe ;;
  *) node_extension=tar.xz; node_executable=bin/node ;;
esac
node_directory="node-v$SCOPE_NODE_VERSION-$SCOPE_NODE_PLATFORM"
node_archive_name="$node_directory.$node_extension"

workspace="$(mktemp -d)"
trap 'rm -rf "$workspace"' EXIT

node_archive="$workspace/$node_archive_name"
mkdir "$workspace/node"
curl --fail --silent --show-error --location \
  "https://nodejs.org/dist/v$SCOPE_NODE_VERSION/$node_archive_name" \
  --output "$node_archive"
if command -v sha256sum >/dev/null 2>&1; then
  actual_node_sha256="$(sha256sum "$node_archive" | awk '{print $1}')"
else
  actual_node_sha256="$(shasum -a 256 "$node_archive" | awk '{print $1}')"
fi
if [[ "$actual_node_sha256" != "$SCOPE_NODE_SHA256" ]]; then
  echo "Node runtime checksum verification failed for $node_archive_name" >&2
  exit 1
fi

case "$node_extension" in
  zip) unzip -q "$node_archive" -d "$workspace/node" ;;
  tar.xz) tar -xJf "$node_archive" -C "$workspace/node" ;;
esac

node_root="$workspace/node/$node_directory"
test -f "$node_root/$node_executable"

bundle="$workspace/bundle"
runtime="$bundle/scope-runtime"
analyzer="$runtime/dependency-analyzer"
licenses="$runtime/licenses"
mkdir -p "$analyzer" "$licenses" "$(dirname "$SCOPE_ARTIFACT")"

scope_name=scope
node_name=node
[[ "$SCOPE_BINARY" == *.exe ]] && scope_name=scope.exe
[[ "$node_executable" == *.exe ]] && node_name=node.exe
cp "$SCOPE_BINARY" "$bundle/$scope_name"
cp "$node_root/$node_executable" "$runtime/$node_name"
chmod 0755 "$bundle/$scope_name" "$runtime/$node_name"

cp dependency-analyzer/analyze.mjs dependency-analyzer/package.json dependency-analyzer/package-lock.json "$analyzer/"
cp -R dependency-analyzer/src dependency-analyzer/node_modules "$analyzer/"
cp LICENSE NOTICE legal/third-party-rust.txt legal/third-party-dependency-analyzer.txt "$licenses/"
cp "$node_root/LICENSE" "$licenses/Node.js-LICENSE"

tar -czf "$SCOPE_ARTIFACT" -C "$bundle" .

artifact_bytes="$(wc -c < "$SCOPE_ARTIFACT" | tr -d '[:space:]')"
max_artifact_bytes="$(jq -er '.max_artifact_bytes' cli/distribution/targets.json)"
if (( artifact_bytes > max_artifact_bytes )); then
  echo "$SCOPE_ARTIFACT is $artifact_bytes bytes, over the $max_artifact_bytes byte cap by $((artifact_bytes - max_artifact_bytes)) bytes" >&2
  exit 1
fi
awk -v name="$(basename "$SCOPE_ARTIFACT")" -v bytes="$artifact_bytes" \
  'BEGIN { printf "%s is %.1f MiB\n", name, bytes / 1048576 }'
