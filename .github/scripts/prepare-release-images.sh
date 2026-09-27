#!/usr/bin/env bash
set -euo pipefail

# Every build writes its own fragment; the merge happens only after all builds
# and their private-registry pull checks have succeeded.
source_sha="${SCOPE_DEPLOYMENT_SOURCE_SHA:?A source revision is required}"
[[ "$source_sha" =~ ^[0-9a-f]{40}$ ]] || { echo 'A full source revision is required.' >&2; exit 2; }
declare -a components=() fragments=()

if [[ "${PREPARE_BACKEND:-0}" == 1 ]]; then
  mkdir -p artifacts/backend/bin
  python3 .github/scripts/extract-railway-release.py backend artifacts/backend/backend-release.tar.gz artifacts/backend/bin
  component_names="$(node .github/scripts/deployment-components.mjs backend-prebuilt)"
  mapfile -t backend_components <<< "$component_names"
  for component in "${backend_components[@]}"; do
    [[ "$component" != git-router || "${PREPARE_ROUTER:-0}" == 1 ]] || continue
    root=".railway-prepared/$component"
    mkdir -p "$root/bin"
    cp artifacts/backend/bin/LICENSE artifacts/backend/bin/NOTICE artifacts/backend/bin/third-party-rust.txt "$root/bin/"
    binary="$(node .github/scripts/deployment-components.mjs field "$component" binary)"
    install -m 0755 "artifacts/backend/bin/$binary" "$root/bin/$binary"
    if [[ "$component" == run-worker ]]; then
      cp -a artifacts/backend/bin/dependency-analyzer "$root/dependency-analyzer"
    elif [[ "$component" == api ]]; then
      : "${SCOPE_MAINTENANCE_BINARY:?API preparation requires the original maintenance binary}"
      install -m 0755 "$SCOPE_MAINTENANCE_BINARY" "$root/bin/scope-maintenance"
    fi
    components+=("$component")
  done
  [[ "${MEDIA_WORKER_IMAGE:-}" =~ ^ghcr\.io/scope-vcs/scope-media-worker@sha256:[0-9a-f]{64}$ ]] || {
    echo 'Media worker requires an immutable image digest.' >&2
    exit 2
  }
  media_worker_service="$(jq -er '.services["media-worker"].id' .github/deployment-services.json)"
  media_fragment=.railway-prepared/media-worker.json
  node .github/scripts/railway-artifact.mjs record "$media_fragment" \
    media-worker "$MEDIA_WORKER_IMAGE" "$source_sha" "$media_worker_service"
fi

if [[ "${PREPARE_WEB:-0}" == 1 ]]; then
  mkdir -p .railway-prepared/web
  python3 .github/scripts/extract-railway-release.py web artifacts/web/web-release.tar.gz .railway-prepared/web
  components+=(web)
fi

if ((${#components[@]} == 0)); then
  echo 'No release images were selected.' >&2
  exit 2
fi

export SCOPE_IMAGE_DEPENDENCY_EPOCH="$(date -u +%G-W%V)"
if [[ "${PREPARE_BACKEND:-0}" == 1 ]]; then
  [[ -d .railway-prepared/api ]] || { echo 'Backend preparation requires the API image context.' >&2; exit 2; }
  # Resolve the pinned Git build once before API and worker package in parallel.
  # This cache tag lives in the already private API image package.
  export SCOPE_GIT_BUILD_CACHE_REF="$(node .github/scripts/railway-artifact.mjs image-repository api):git-buildcache"
  git_version="$(jq -er '.git.version' dev/tool-versions.json)"
  git_source_sha256="$(jq -er '.git.sourceSha256' dev/tool-versions.json)"
  cp deploy/railway/install-git.sh .railway-prepared/api/install-git.sh
  docker buildx build --platform linux/amd64 --provenance=false \
    --file deploy/railway/prebuilt.Dockerfile --target git-builder \
    --output type=cacheonly \
    --cache-from "type=registry,ref=$SCOPE_GIT_BUILD_CACHE_REF" \
    --cache-to "type=registry,ref=$SCOPE_GIT_BUILD_CACHE_REF,mode=max,image-manifest=true,oci-mediatypes=true" \
    --build-arg INSTALL_GIT=1 \
    --build-arg "GIT_VERSION=$git_version" --build-arg "GIT_SOURCE_SHA256=$git_source_sha256" \
    --build-arg "IMAGE_DEPENDENCY_EPOCH=$SCOPE_IMAGE_DEPENDENCY_EPOCH" \
    .railway-prepared/api
  node .github/scripts/railway-artifact.mjs verify-private-package api
fi

for component in "${components[@]}"; do
  fragments+=(".railway-prepared/$component.json")
done
bash .github/scripts/run-bounded-image-builds.sh .github/scripts/prepare-railway-artifact.sh "${components[@]}"

if [[ "${PREPARE_BACKEND:-0}" == 1 ]]; then
  components+=(media-worker)
  fragments+=("$media_fragment")
fi
node .github/scripts/merge-prepared-release.mjs prepared-release.json "$source_sha" \
  "${components[*]}" "${fragments[@]}"
