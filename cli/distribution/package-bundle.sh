#!/usr/bin/env bash
set -euo pipefail

: "${SCOPE_BINARY:?Set SCOPE_BINARY to the built scope executable}"
: "${SCOPE_ARTIFACT:?Set SCOPE_ARTIFACT to the output archive path}"
: "${SCOPE_NODE_ARCHIVE:?Set SCOPE_NODE_ARCHIVE to the pinned Node archive name}"
: "${SCOPE_NODE_SHA256:?Set SCOPE_NODE_SHA256 to the pinned Node archive checksum}"
: "${SCOPE_NODE_DIRECTORY:?Set SCOPE_NODE_DIRECTORY to the Node archive root}"
: "${SCOPE_NODE_EXECUTABLE:?Set SCOPE_NODE_EXECUTABLE to the executable within that root}"

workspace="$(mktemp -d)"
trap 'rm -rf "$workspace"' EXIT

node_archive="$workspace/$SCOPE_NODE_ARCHIVE"
mkdir "$workspace/node"
curl --fail --silent --show-error --location \
  "https://nodejs.org/dist/v24.21.0/$SCOPE_NODE_ARCHIVE" \
  --output "$node_archive"
if command -v sha256sum >/dev/null 2>&1; then
  actual_node_sha256="$(sha256sum "$node_archive" | awk '{print $1}')"
else
  actual_node_sha256="$(shasum -a 256 "$node_archive" | awk '{print $1}')"
fi
if [[ "$actual_node_sha256" != "$SCOPE_NODE_SHA256" ]]; then
  echo "Node runtime checksum verification failed for $SCOPE_NODE_ARCHIVE" >&2
  exit 1
fi

case "$SCOPE_NODE_ARCHIVE" in
  *.zip) unzip -q "$node_archive" -d "$workspace/node" ;;
  *.tar.xz) tar -xJf "$node_archive" -C "$workspace/node" ;;
  *) echo "Unsupported Node archive: $SCOPE_NODE_ARCHIVE" >&2; exit 1 ;;
esac

node_root="$workspace/node/$SCOPE_NODE_DIRECTORY"
test -x "$node_root/$SCOPE_NODE_EXECUTABLE" || test -f "$node_root/$SCOPE_NODE_EXECUTABLE"

bundle="$workspace/bundle"
runtime="$bundle/scope-runtime"
analyzer="$runtime/dependency-analyzer"
licenses="$runtime/licenses"
mkdir -p "$analyzer" "$licenses" "$(dirname "$SCOPE_ARTIFACT")"

scope_name=scope
node_name=node
[[ "$SCOPE_BINARY" == *.exe ]] && scope_name=scope.exe
[[ "$SCOPE_NODE_EXECUTABLE" == *.exe ]] && node_name=node.exe
cp "$SCOPE_BINARY" "$bundle/$scope_name"
cp "$node_root/$SCOPE_NODE_EXECUTABLE" "$runtime/$node_name"
chmod 0755 "$bundle/$scope_name" "$runtime/$node_name"

cp dependency-analyzer/analyze.mjs dependency-analyzer/package.json dependency-analyzer/package-lock.json "$analyzer/"
cp -R dependency-analyzer/src dependency-analyzer/node_modules "$analyzer/"
cp LICENSE NOTICE legal/third-party-rust.txt legal/third-party-dependency-analyzer.txt "$licenses/"
cp "$node_root/LICENSE" "$licenses/Node.js-LICENSE"

tar -czf "$SCOPE_ARTIFACT" -C "$bundle" .
