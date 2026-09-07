#!/usr/bin/env bash
set -euo pipefail

component="${1:?usage: prepare-railway-artifact.sh <component> <context-root> <manifest-path>}"
context_root="${2:?context-root is required}"
release_path="${3:?manifest-path is required}"
source_sha="${SCOPE_DEPLOYMENT_SOURCE_SHA:-${GITHUB_SHA:-}}"
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || { echo 'A full source revision is required.' >&2; exit 2; }
service_id="$(jq -er --arg component "$component" '.services[$component].id' "${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}")"
image_repository="${SCOPE_ARTIFACT_IMAGE_REPOSITORY:-ghcr.io/${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}/railway-$component}"
image_repository="${image_repository,,}"
image_tag="$image_repository:$component-$source_sha-${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-1}"
metadata="$(mktemp)"
pull_config="$(mktemp -d)"
trap 'rm -f "$metadata"; rm -rf "$pull_config"' EXIT

dockerfile=deploy/railway/prebuilt.Dockerfile
install_git=0
binary=""
case "$component" in
  api) install_git=1; binary=scope-vcs ;;
  worker) install_git=1; binary=scope-worker ;;
  cache) binary=scope-cache-service ;;
  router) binary=scope-repo-router ;;
  cli) binary=scope-cli-service ;;
  web) dockerfile=deploy/railway/web.Dockerfile; test -s "$context_root/.output/server/index.mjs" ;;
  *) echo "Unknown release component $component" >&2; exit 2 ;;
esac
if [[ "$component" != web ]]; then
  test -x "$context_root/bin/$binary"
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
if [[ -n "${SCOPE_RAILWAY_REGISTRY_USERNAME:-}" || -n "${SCOPE_RAILWAY_REGISTRY_PASSWORD:-}" ]]; then
  : "${SCOPE_RAILWAY_REGISTRY_USERNAME:?Registry username is required}"
  : "${SCOPE_RAILWAY_REGISTRY_PASSWORD:?Registry password is required}"
  printf '%s' "$SCOPE_RAILWAY_REGISTRY_PASSWORD" |
    DOCKER_CONFIG="$pull_config" docker login "${image_repository%%/*}" \
      --username "$SCOPE_RAILWAY_REGISTRY_USERNAME" --password-stdin >/dev/null
fi
DOCKER_CONFIG="$pull_config" docker manifest inspect "$image" >/dev/null || {
  echo 'Prepared image is not pullable anonymously or with the durable Railway registry credentials. Configure registry credentials before cutover.' >&2
  exit 1
}
node .github/scripts/railway-artifact.mjs record "$release_path" "$component" "$image" "$source_sha" "$service_id"
