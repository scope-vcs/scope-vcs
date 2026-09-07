#!/usr/bin/env bash
set -euo pipefail

release_path="${1:?usage: extract-railway-maintenance.sh <prepared-manifest> <binary-destination>}"
destination="${2:?binary-destination is required}"
source_sha="${SCOPE_DEPLOYMENT_SOURCE_SHA:-$(jq -er '.sourceSha' "$release_path")}"
node .github/scripts/railway-artifact.mjs validate "$release_path" "$source_sha" api >/dev/null
image="$(jq -er '.components.api.image' "$release_path")"
jq -e '.maintenanceSha256 | strings | test("^[0-9a-f]{64}$")' "$release_path" >/dev/null
if [[ -n "${SCOPE_RAILWAY_REGISTRY_USERNAME:-}" || -n "${SCOPE_RAILWAY_REGISTRY_PASSWORD:-}" ]]; then
  : "${SCOPE_RAILWAY_REGISTRY_USERNAME:?Registry username is required}"
  : "${SCOPE_RAILWAY_REGISTRY_PASSWORD:?Registry password is required}"
fi
if [[ -L "$destination" || -d "$destination" ]]; then
  echo 'Maintenance destination must be a regular file path.' >&2
  exit 2
fi

umask 077
mkdir -p -- "$(dirname -- "$destination")"
extraction_root="$(mktemp -d "$(dirname -- "$destination")/.scope-maintenance.XXXXXX")"
pull_config="$extraction_root/docker-config"
mkdir "$pull_config"
container_name="scope-maintenance-$(basename "$extraction_root")"
docker_with_config() { DOCKER_CONFIG="$pull_config" docker "$@"; }
cleanup() {
  docker_with_config container rm --volumes "$container_name" >/dev/null 2>&1 || true
  rm -rf -- "$extraction_root"
}
trap cleanup EXIT

# Use only anonymous access or the durable provider pull credential. The runner's
# publishing login may already have expired when a closed cutover is recovered.
if [[ -n "${SCOPE_RAILWAY_REGISTRY_USERNAME:-}" ]]; then
  printf '%s' "$SCOPE_RAILWAY_REGISTRY_PASSWORD" |
    docker_with_config login "${image%%/*}" \
      --username "$SCOPE_RAILWAY_REGISTRY_USERNAME" --password-stdin >/dev/null
fi
docker_with_config pull --platform linux/amd64 "$image" >/dev/null
# Creating a stopped container materializes its filesystem without executing any
# candidate binary, image entrypoint, or package script. Never start this container.
docker_with_config create --name "$container_name" --network none \
  --entrypoint /bin/false "$image" >/dev/null
docker_with_config cp "$container_name:/app/bin/scope-maintenance" "$extraction_root/scope-maintenance"
if [[ ! -f "$extraction_root/scope-maintenance" || -L "$extraction_root/scope-maintenance" ]]; then
  echo 'Prepared API image does not contain a regular maintenance binary.' >&2
  exit 1
fi
node .github/scripts/railway-artifact.mjs verify-maintenance \
  "$release_path" "$extraction_root/scope-maintenance" >/dev/null
chmod 0755 "$extraction_root/scope-maintenance"
mv -T -- "$extraction_root/scope-maintenance" "$destination"
printf 'Restored verified maintenance binary from %s.\n' "$image"
