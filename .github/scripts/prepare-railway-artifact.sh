#!/usr/bin/env bash
set -euo pipefail

component="${1:?usage: prepare-railway-artifact.sh <component> <context-root> <manifest-path>}"
context_root="${2:?context-root is required}"
release_path="${3:?manifest-path is required}"
source_sha="${SCOPE_DEPLOYMENT_SOURCE_SHA:-${GITHUB_SHA:-}}"
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || { echo 'A full source revision is required.' >&2; exit 2; }
: "${SCOPE_RAILWAY_REGISTRY_USERNAME:?Private image preparation requires durable registry username}"
: "${SCOPE_RAILWAY_REGISTRY_PASSWORD:?Private image preparation requires durable registry password}"
: "${GITHUB_TOKEN:?Private image preparation requires publishing token for visibility verification}"
service_id="$(jq -er --arg component "$component" '.services[$component].id' "${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}")"
image_repository="$(node .github/scripts/railway-artifact.mjs image-repository "$component")"
image_tag="$image_repository:$component-$source_sha-${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"
metadata="$(mktemp)"
pull_config="$(mktemp -d)"
trap 'rm -f "$metadata"; rm -rf "$pull_config"' EXIT

definition="$(node .github/scripts/deployment-components.mjs describe "$component")"
dockerfile="$(jq -er '.artifact.dockerfile' <<< "$definition")"
install_git="$(jq -r 'if .artifact.installGit then 1 else 0 end' <<< "$definition")"
binary="$(jq -r '.artifact.binary // ""' <<< "$definition")"
if [[ "$(jq -r '.artifact.kind' <<< "$definition")" == web ]]; then
  test -s "$context_root/.output/server/index.mjs"
else
  test -x "$context_root/bin/$binary"
fi
if [[ "$component" == api ]]; then
  : "${SCOPE_MAINTENANCE_BINARY:?API preparation requires the original maintenance binary}"
  test -f "$context_root/bin/scope-maintenance"
  test ! -L "$context_root/bin/scope-maintenance"
  cmp -- "$SCOPE_MAINTENANCE_BINARY" "$context_root/bin/scope-maintenance" || {
    echo 'API image maintenance binary differs from the prepared release binary.' >&2
    exit 1
  }
fi
printf '%s\n' "$source_sha" > "$context_root/.scope-deployment-sha"
docker buildx build --platform linux/amd64 --provenance=false --push \
  --file "$dockerfile" --tag "$image_tag" --metadata-file "$metadata" \
  --label "org.opencontainers.image.revision=$source_sha" \
  --label "org.opencontainers.image.source=https://github.com/${GITHUB_REPOSITORY}" \
  --build-arg "INSTALL_GIT=$install_git" --build-arg "BINARY=$binary" "$context_root"
digest="$(jq -er '."containerimage.digest"' "$metadata")"
[[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo 'Build did not publish an immutable image digest.' >&2; exit 1; }
image="$image_repository@$digest"

# Verify that Railway can pull after the workflow token expires. A clean Docker
# config prevents accidental validation with the short-lived publishing token.
printf '%s' "$SCOPE_RAILWAY_REGISTRY_PASSWORD" |
  DOCKER_CONFIG="$pull_config" docker login "${image_repository%%/*}" \
    --username "$SCOPE_RAILWAY_REGISTRY_USERNAME" --password-stdin >/dev/null
DOCKER_CONFIG="$pull_config" docker manifest inspect "$image" >/dev/null || {
  echo 'Prepared private image is not pullable with the durable Railway registry credentials.' >&2
  exit 1
}
node .github/scripts/railway-artifact.mjs verify-private-package "$component"
node .github/scripts/railway-artifact.mjs record "$release_path" "$component" "$image" "$source_sha" "$service_id"
