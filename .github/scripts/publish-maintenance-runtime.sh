#!/usr/bin/env bash
set -euo pipefail
umask 077
: "${SCOPE_RAILWAY_REGISTRY_USERNAME:?Durable registry username is required}"
: "${SCOPE_RAILWAY_REGISTRY_PASSWORD:?Durable registry password is required}"
[[ "${GITHUB_SHA:-}" =~ ^[a-f0-9]{40}$ ]]
[[ "${GITHUB_RUN_ID:-}" =~ ^[0-9]+$ && "${GITHUB_RUN_ATTEMPT:-}" =~ ^[0-9]+$ ]]
output="${1:-maintenance-runtime.json}"
manifest="${SCOPE_DEPLOYMENT_MANIFEST:-.github/deployment-services.json}"
workspace="$(mktemp -d)"
trap 'rm -rf -- "$workspace"' EXIT
trap 'exit 1' HUP INT TERM
node .github/scripts/maintenance-runtime.mjs source "$workspace/source.json"
mkdir -p "$workspace/context/bin" "$workspace/pull-config"
cp deploy/railway/install-git.sh "$workspace/context/install-git.sh"
git_version="$(jq -er '.git.version' dev/tool-versions.json)"
git_source_sha256="$(jq -er '.git.sourceSha256' dev/tool-versions.json)"
bash .github/scripts/extract-railway-maintenance.sh "$workspace/source.json" "$workspace/context/bin/scope-maintenance"
# The running image, extracted bytes and operator's source identity must agree.
image="$(jq -er '.components.api.image' "$workspace/source.json")"
[[ "$(docker image inspect "$image" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}')" == "$CURRENT_API_SOURCE_SHA" ]]
repository="ghcr.io/${GITHUB_REPOSITORY,,}/$(jq -er '.railway.releaseImagePrefix' "$manifest")-maintenance"
tag="$repository:runtime-$GITHUB_SHA-$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT"
node .github/scripts/maintenance-runtime.mjs verify-publish-target
docker buildx build --platform linux/amd64 --provenance=false --push \
  --file deploy/railway/maintenance.Dockerfile --tag "$tag" --metadata-file "$workspace/metadata.json" \
  --label "org.opencontainers.image.source=https://github.com/$GITHUB_REPOSITORY" \
  --label "org.opencontainers.image.revision=$GITHUB_SHA" \
  --build-arg "GIT_VERSION=$git_version" --build-arg "GIT_SOURCE_SHA256=$git_source_sha256" \
  "$workspace/context"
digest="$(jq -er '."containerimage.digest" | select(test("^sha256:[a-f0-9]{64}$"))' "$workspace/metadata.json")"
image="$repository@$digest"
printf '%s' "$SCOPE_RAILWAY_REGISTRY_PASSWORD" | \
  DOCKER_CONFIG="$workspace/pull-config" docker login ghcr.io --username "$SCOPE_RAILWAY_REGISTRY_USERNAME" --password-stdin >/dev/null
DOCKER_CONFIG="$workspace/pull-config" docker manifest inspect "$image" >/dev/null
node .github/scripts/maintenance-runtime.mjs verify-package
jq -n --arg image "$image" --arg runtimeSourceSha "$GITHUB_SHA" --slurpfile source "$workspace/source.json" \
  '{schemaVersion:1,image:$image,runtimeSourceSha:$runtimeSourceSha,apiImage:$source[0].components.api.image,
    apiSourceSha:$source[0].sourceSha,maintenanceSha256:$source[0].maintenanceSha256}' > "$output"
